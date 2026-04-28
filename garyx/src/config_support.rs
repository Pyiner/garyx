use std::path::PathBuf;

use garyx_models::config_loader::{
    ConfigLoadFailure, ConfigLoadOptions, ConfigRuntimeOverrides, LoadedConfig, PreparedConfigPath,
    default_config_path, load_config, prepare_config_path_for_io,
};

pub(crate) fn default_config_path_buf() -> PathBuf {
    default_config_path()
}

pub(crate) fn default_config_path_string() -> String {
    default_config_path_buf().to_string_lossy().to_string()
}

pub(crate) fn prepare_config_path_for_io_buf(config_path: &str) -> PreparedConfigPath {
    prepare_config_path_for_io(PathBuf::from(config_path), default_config_path_buf())
}

pub(crate) fn load_config_or_default(
    path: &str,
    runtime_overrides: ConfigRuntimeOverrides,
) -> Result<LoadedConfig, ConfigLoadFailure> {
    let options = ConfigLoadOptions {
        default_path: default_config_path_buf(),
        runtime_overrides,
    };
    load_config(path, &options)
}

pub(crate) fn print_diagnostics(diagnostics: &garyx_models::config_loader::ConfigDiagnostics) {
    for warning in &diagnostics.warnings {
        if let Some(path) = &warning.path {
            eprintln!("[warn][{}] {} ({path})", warning.code, warning.message);
        } else {
            eprintln!("[warn][{}] {}", warning.code, warning.message);
        }
    }
}

pub(crate) fn print_errors(diagnostics: &garyx_models::config_loader::ConfigDiagnostics) {
    for error in &diagnostics.errors {
        if let Some(path) = &error.path {
            eprintln!("[error][{}] {} ({path})", error.code, error.message);
        } else {
            eprintln!("[error][{}] {}", error.code, error.message);
        }
    }
}
