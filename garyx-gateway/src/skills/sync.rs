use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
#[cfg(unix)]
use std::os::unix::fs as unix_fs;
#[cfg(windows)]
use std::os::windows::fs as windows_fs;
use std::path::{Path, PathBuf};

use garyx_models::local_paths::skills_sync_state_path_for_gary_home;
use serde::{Deserialize, Serialize};

use super::{SkillStoreError, is_valid_skill_id};

#[derive(Debug, Default, Deserialize, Serialize)]
struct ManagedSkillSyncState {
    #[serde(default)]
    managed_ids: Vec<String>,
}

pub(super) fn sync_external_user_skills(
    user_dir: &Path,
    state: &HashMap<String, bool>,
) -> Result<(), SkillStoreError> {
    let Some(home) = sync_home(user_dir) else {
        return Ok(());
    };
    sync_external_user_skills_in_home(user_dir, state, &home)
}

fn sync_external_user_skills_in_home(
    user_dir: &Path,
    state: &HashMap<String, bool>,
    home: &Path,
) -> Result<(), SkillStoreError> {
    let managed_state_path = skills_sync_state_path_for_gary_home(&home.join(".garyx"));
    let previous_managed_ids = read_managed_ids(&managed_state_path)?;
    let enabled_ids = enabled_user_skill_ids(user_dir, state)?;
    let targets = [
        home.join(".claude").join("skills"),
        home.join(".codex").join("skills"),
    ];

    for target_root in &targets {
        sync_target_root(user_dir, target_root, &previous_managed_ids, &enabled_ids)?;
    }

    write_managed_ids(&managed_state_path, &enabled_ids)
}

fn sync_home(user_dir: &Path) -> Option<PathBuf> {
    let skills_name = user_dir.file_name()?.to_str()?;
    let gary_dir = user_dir.parent()?;
    let gary_name = gary_dir.file_name()?.to_str()?;
    if skills_name == "skills" && gary_name == ".garyx" {
        return gary_dir.parent().map(Path::to_path_buf);
    }
    None
}

fn read_managed_ids(path: &Path) -> Result<HashSet<String>, SkillStoreError> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(error) => return Err(SkillStoreError::Io(error)),
    };
    if raw.trim().is_empty() {
        return Ok(HashSet::new());
    }

    match serde_json::from_str::<ManagedSkillSyncState>(&raw) {
        Ok(state) => Ok(state.managed_ids.into_iter().collect()),
        Err(error) => Err(SkillStoreError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to parse {}: {error}", path.display()),
        ))),
    }
}

fn write_managed_ids(path: &Path, ids: &HashSet<String>) -> Result<(), SkillStoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut managed_ids = ids.iter().cloned().collect::<Vec<_>>();
    managed_ids.sort();
    let raw = serde_json::to_vec_pretty(&ManagedSkillSyncState { managed_ids })
        .map_err(io::Error::other)?;
    fs::write(path, raw)?;
    Ok(())
}

fn enabled_user_skill_ids(
    user_dir: &Path,
    state: &HashMap<String, bool>,
) -> Result<HashSet<String>, SkillStoreError> {
    if !user_dir.is_dir() {
        return Ok(HashSet::new());
    }

    let mut ids = HashSet::new();
    for entry in fs::read_dir(user_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let id = entry.file_name().to_string_lossy().to_string();
        if !is_valid_skill_id(&id) || !entry.path().join("SKILL.md").is_file() {
            continue;
        }
        if state.get(&id).copied().unwrap_or(true) {
            ids.insert(id);
        }
    }

    Ok(ids)
}

fn sync_target_root(
    user_dir: &Path,
    target_root: &Path,
    previous_managed_ids: &HashSet<String>,
    enabled_ids: &HashSet<String>,
) -> Result<(), SkillStoreError> {
    fs::create_dir_all(target_root)?;

    for id in previous_managed_ids {
        if enabled_ids.contains(id) {
            continue;
        }
        remove_path_if_exists(&target_root.join(id))?;
    }

    for id in enabled_ids {
        let source_dir = user_dir.join(id);
        if !source_dir.join("SKILL.md").is_file() {
            continue;
        }
        let target_dir = target_root.join(id);
        remove_path_if_exists(&target_dir)?;
        symlink_skill_dir(&source_dir, &target_dir)?;
    }

    Ok(())
}

fn remove_path_if_exists(path: &Path) -> Result<(), SkillStoreError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(SkillStoreError::Io(error)),
    };

    if metadata.file_type().is_symlink() {
        return remove_symlink(path);
    }
    if metadata.file_type().is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn remove_symlink(path: &Path) -> Result<(), SkillStoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::IsADirectory | io::ErrorKind::PermissionDenied
            ) =>
        {
            fs::remove_dir(path)?;
            Ok(())
        }
        Err(error) => Err(SkillStoreError::Io(error)),
    }
}

fn symlink_skill_dir(source: &Path, target: &Path) -> Result<(), SkillStoreError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    #[cfg(unix)]
    {
        unix_fs::symlink(source, target)?;
    }

    #[cfg(windows)]
    {
        windows_fs::symlink_dir(source, target)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
