use std::fs;
use std::path::{Path, PathBuf};

use super::diagnostics::ConfigDiagnostics;

#[derive(Debug, Clone)]
pub struct PreparedConfigPath {
    pub active_path: PathBuf,
    pub diagnostics: ConfigDiagnostics,
}

pub fn default_config_path() -> PathBuf {
    match home_dir() {
        Some(home) => home.join(".garyx").join("garyx.json"),
        None => PathBuf::from("garyx.json"),
    }
}

pub fn prepare_config_path_for_io(
    requested_path: impl AsRef<Path>,
    default_path: impl AsRef<Path>,
) -> PreparedConfigPath {
    let requested_path = requested_path.as_ref().to_path_buf();
    let default_path = default_path.as_ref();
    let mut diagnostics = ConfigDiagnostics::default();

    if requested_path != default_path {
        return PreparedConfigPath {
            active_path: requested_path,
            diagnostics,
        };
    }

    let Some(legacy_path) = legacy_default_config_path() else {
        return PreparedConfigPath {
            active_path: requested_path,
            diagnostics,
        };
    };

    if requested_path.exists() || !legacy_path.exists() {
        return PreparedConfigPath {
            active_path: requested_path,
            diagnostics,
        };
    }

    match migrate_legacy_config(&legacy_path, &requested_path) {
        Ok(()) => {
            diagnostics.push_warning(
                "CONFIG_PATH_MIGRATED",
                format!(
                    "migrated legacy config from {} to {}",
                    legacy_path.display(),
                    requested_path.display()
                ),
                None::<String>,
            );
            PreparedConfigPath {
                active_path: requested_path,
                diagnostics,
            }
        }
        Err(error) => {
            diagnostics.push_warning(
                "CONFIG_PATH_LEGACY_FALLBACK",
                format!(
                    "failed to migrate legacy config from {} to {}: {error}; using legacy path",
                    legacy_path.display(),
                    requested_path.display()
                ),
                None::<String>,
            );
            PreparedConfigPath {
                active_path: legacy_path,
                diagnostics,
            }
        }
    }
}

fn migrate_legacy_config(legacy_path: &Path, current_path: &Path) -> std::io::Result<()> {
    if let Some(parent) = current_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(legacy_path, current_path)?;
    Ok(())
}

fn legacy_default_config_path() -> Option<PathBuf> {
    // Check ~/.gary/gary.json first (most recent legacy), then ~/gary/gary.json (oldest legacy)
    let home = home_dir()?;
    let gary = home.join(".gary").join("gary.json");
    if gary.exists() {
        return Some(gary);
    }
    let oldest = home.join("gary").join("gary.json");
    if oldest.exists() {
        return Some(oldest);
    }
    // Return the most likely legacy path for migration messaging
    Some(gary)
}

fn home_dir() -> Option<PathBuf> {
    crate::local_paths::home_dir()
}
