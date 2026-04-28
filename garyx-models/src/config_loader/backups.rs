use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn backup_directory(config_path: &Path) -> io::Result<PathBuf> {
    let parent = config_path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "config path has no parent"))?;
    Ok(parent.join("backups"))
}

/// Create a timestamped backup of the given config file.
///
/// Returns the path of the backup file. If the config file does not exist,
/// returns `Ok(None)`.
pub fn backup_config(config_path: &Path) -> io::Result<Option<PathBuf>> {
    if !config_path.exists() {
        return Ok(None);
    }
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    // Use seconds + sub-second nanos to avoid collisions within the same second.
    let tag = format!("{}.{}", dur.as_secs(), dur.subsec_nanos());
    let backup_dir = backup_directory(config_path)?;
    fs::create_dir_all(&backup_dir)?;
    let file_name = config_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("gary.json");
    let backup = backup_dir.join(format!("{file_name}.backup.{tag}"));
    fs::copy(config_path, &backup)?;
    Ok(Some(backup))
}

/// List available timestamped backups for a config file, newest first.
pub fn list_backups(config_path: &Path) -> io::Result<Vec<PathBuf>> {
    let parent = config_path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "config path has no parent"))?;

    let stem = config_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("gary.json");

    // Match both dedicated-backup-dir timestamped files and legacy `.bak(.N)` styles.
    let prefix_ts = format!("{stem}.backup.");
    let prefix_bak = format!("{stem}.bak");

    let mut backups = Vec::new();
    let dedicated_dir = backup_directory(config_path)?;
    if dedicated_dir.exists() {
        for entry in fs::read_dir(&dedicated_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(&prefix_ts) {
                backups.push(entry.path());
            }
        }
    }
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with(&prefix_ts) || name_str.starts_with(&prefix_bak) {
            backups.push(entry.path());
        }
    }

    // Sort descending by modification time (newest first).
    backups.sort_by(|a, b| {
        let ma = fs::metadata(a).and_then(|m| m.modified()).ok();
        let mb = fs::metadata(b).and_then(|m| m.modified()).ok();
        mb.cmp(&ma)
    });

    Ok(backups)
}

/// Restore a config from a backup file. Backs up the current config first.
pub fn restore_config(backup_path: &Path, config_path: &Path) -> io::Result<()> {
    if !backup_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("backup file not found: {}", backup_path.display()),
        ));
    }
    // Back up current before restoring.
    backup_config(config_path)?;
    fs::copy(backup_path, config_path)?;
    Ok(())
}
