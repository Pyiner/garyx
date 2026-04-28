use std::fs;
use std::io;
use std::path::Path;

use serde_json::Value;

use crate::config::GaryxConfig;

use super::backups::{backup_config, list_backups};
use super::pipeline::strip_redundant_config_fields;

#[derive(Debug, Clone)]
pub struct ConfigWriteOptions {
    pub backup_keep: usize,
    pub mode: Option<u32>,
}

impl Default for ConfigWriteOptions {
    fn default() -> Self {
        Self {
            backup_keep: 1,
            mode: Some(0o600),
        }
    }
}

pub fn write_config_atomic(
    path: impl AsRef<Path>,
    config: &GaryxConfig,
    options: &ConfigWriteOptions,
) -> io::Result<()> {
    let value = serde_json::to_value(config).map_err(io::Error::other)?;
    write_config_value_atomic(path, &value, options)
}

pub fn write_config_value_atomic(
    path: impl AsRef<Path>,
    value: &Value,
    options: &ConfigWriteOptions,
) -> io::Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    create_timestamped_backup(path, options.backup_keep)?;

    let mut tmp_name = format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("config"),
        std::process::id()
    );
    if tmp_name.is_empty() {
        tmp_name = ".config.tmp".to_owned();
    }
    let tmp_path = path.with_file_name(tmp_name);

    let mut sanitized = value.clone();
    strip_redundant_config_fields(&mut sanitized);
    let bytes = serde_json::to_vec_pretty(&sanitized).map_err(io::Error::other)?;
    fs::write(&tmp_path, bytes)?;

    #[cfg(unix)]
    if let Some(mode) = options.mode {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp_path, fs::Permissions::from_mode(mode))?;
    }

    fs::rename(&tmp_path, path)?;

    #[cfg(unix)]
    if let Some(mode) = options.mode {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }

    Ok(())
}
fn create_timestamped_backup(path: &Path, keep: usize) -> io::Result<()> {
    if keep == 0 {
        return Ok(());
    }

    backup_config(path)?;

    let backups = list_backups(path)?;
    for stale in backups.into_iter().skip(keep) {
        if stale.exists() {
            fs::remove_file(stale)?;
        }
    }

    Ok(())
}
