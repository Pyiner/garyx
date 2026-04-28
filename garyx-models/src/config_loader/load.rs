use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::config::GaryxConfig;

use super::diagnostics::ConfigDiagnostics;
use super::includes::process_includes;
use super::paths::{default_config_path, prepare_config_path_for_io};
use super::pipeline::{
    merge_defaults, normalize_paths, resolve_config_path, strip_legacy_config_fields,
    substitute_env_in_value,
};

#[derive(Debug, Clone, Default)]
pub struct ConfigRuntimeOverrides {
    pub gateway_host: Option<String>,
    pub gateway_port: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct ConfigLoadOptions {
    pub default_path: PathBuf,
    pub runtime_overrides: ConfigRuntimeOverrides,
}

impl Default for ConfigLoadOptions {
    fn default() -> Self {
        Self {
            default_path: default_config_path(),
            runtime_overrides: ConfigRuntimeOverrides::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub path: PathBuf,
    pub from_file: bool,
    pub config: GaryxConfig,
    pub diagnostics: ConfigDiagnostics,
}

#[derive(Debug, Clone)]
pub struct ConfigLoadFailure {
    pub path: PathBuf,
    pub diagnostics: ConfigDiagnostics,
}

impl std::fmt::Display for ConfigLoadFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(first) = self.diagnostics.errors.first() {
            write!(f, "{}: {}", first.code, first.message)
        } else {
            write!(f, "configuration load failed")
        }
    }
}

impl std::error::Error for ConfigLoadFailure {}

pub fn load_config(
    input_path: impl AsRef<Path>,
    options: &ConfigLoadOptions,
) -> Result<LoadedConfig, ConfigLoadFailure> {
    let mut diagnostics = ConfigDiagnostics::default();

    let resolved_path = resolve_config_path(input_path.as_ref(), &options.default_path);
    let prepared_path = prepare_config_path_for_io(&resolved_path, &options.default_path);
    diagnostics
        .warnings
        .extend(prepared_path.diagnostics.warnings);
    let resolved_path = prepared_path.active_path;
    let base_dir = resolved_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let mut root: Value;
    let from_file = resolved_path.exists();

    if from_file {
        let raw = match fs::read_to_string(&resolved_path) {
            Ok(s) => s,
            Err(e) => {
                diagnostics.push_error(
                    "CONFIG_IO_READ",
                    format!(
                        "failed to read config file {}: {e}",
                        resolved_path.display()
                    ),
                    None::<String>,
                );
                return Err(ConfigLoadFailure {
                    path: resolved_path,
                    diagnostics,
                });
            }
        };

        root = match serde_json::from_str::<Value>(&raw) {
            Ok(v) => v,
            Err(e) => {
                diagnostics.push_error(
                    "CONFIG_JSON_PARSE",
                    format!("invalid JSON: {e}"),
                    None::<String>,
                );
                return Err(ConfigLoadFailure {
                    path: resolved_path,
                    diagnostics,
                });
            }
        };

        if !root.is_object() {
            diagnostics.push_error(
                "CONFIG_ROOT_TYPE",
                "config root must be a JSON object",
                Some("$"),
            );
            return Err(ConfigLoadFailure {
                path: resolved_path,
                diagnostics,
            });
        }
    } else {
        diagnostics.push_warning(
            "CONFIG_FILE_MISSING",
            format!(
                "config file {} not found, using defaults",
                resolved_path.display()
            ),
            None::<String>,
        );
        root = match serde_json::to_value(GaryxConfig::default()) {
            Ok(value) => value,
            Err(e) => {
                diagnostics.push_error(
                    "CONFIG_DEFAULT_SERIALIZE",
                    format!("failed to serialize default config: {e}"),
                    None::<String>,
                );
                return Err(ConfigLoadFailure {
                    path: resolved_path,
                    diagnostics,
                });
            }
        };
    }

    // Process $include directives before env substitution so included
    // fragments can themselves contain ${VAR} references.
    process_includes(&mut root, &base_dir, &mut diagnostics);
    if diagnostics.has_errors() {
        return Err(ConfigLoadFailure {
            path: resolved_path,
            diagnostics,
        });
    }

    substitute_env_in_value(&mut root, "$", &mut diagnostics);
    strip_legacy_config_fields(&mut root, Some(&mut diagnostics));

    let defaults = match serde_json::to_value(GaryxConfig::default()) {
        Ok(value) => value,
        Err(e) => {
            diagnostics.push_error(
                "CONFIG_DEFAULT_SERIALIZE",
                format!("failed to serialize default config: {e}"),
                None::<String>,
            );
            return Err(ConfigLoadFailure {
                path: resolved_path,
                diagnostics,
            });
        }
    };
    root = merge_defaults(&defaults, &root);

    normalize_paths(&mut root, &base_dir, &mut diagnostics);
    apply_runtime_overrides(&mut root, &options.runtime_overrides, &mut diagnostics);

    let config = match serde_json::from_value::<GaryxConfig>(root) {
        Ok(cfg) => cfg,
        Err(e) => {
            diagnostics.push_error(
                "CONFIG_DESERIALIZE",
                format!("config validation failed: {e}"),
                None::<String>,
            );
            return Err(ConfigLoadFailure {
                path: resolved_path,
                diagnostics,
            });
        }
    };

    if diagnostics.has_errors() {
        return Err(ConfigLoadFailure {
            path: resolved_path,
            diagnostics,
        });
    }

    Ok(LoadedConfig {
        path: resolved_path,
        from_file,
        config,
        diagnostics,
    })
}

fn apply_runtime_overrides(
    root: &mut Value,
    options: &ConfigRuntimeOverrides,
    diagnostics: &mut ConfigDiagnostics,
) {
    let mut env_overrides = ConfigRuntimeOverrides::default();

    if let Ok(host_raw) = std::env::var("GARYX_GATEWAY_HOST") {
        let host = host_raw.trim();
        if !host.is_empty() {
            env_overrides.gateway_host = Some(host.to_owned());
        }
    }

    if let Ok(port_raw) = std::env::var("GARYX_GATEWAY_PORT") {
        let port_raw = port_raw.trim();
        if !port_raw.is_empty() {
            match port_raw.parse::<u16>() {
                Ok(port) => env_overrides.gateway_port = Some(port),
                Err(_) => diagnostics.push_warning(
                    "CONFIG_OVERRIDE_PORT_PARSE",
                    format!("ignoring invalid GARYX_GATEWAY_PORT value: {port_raw}"),
                    Some("$.gateway.port"),
                ),
            }
        }
    }

    let final_host = options.gateway_host.clone().or(env_overrides.gateway_host);
    let final_port = options.gateway_port.or(env_overrides.gateway_port);

    if let Some(host) = final_host {
        set_object_path(root, &["gateway", "host"], Value::String(host));
    }
    if let Some(port) = final_port {
        set_object_path(
            root,
            &["gateway", "port"],
            Value::Number(serde_json::Number::from(port)),
        );
    }
}

fn set_object_path(root: &mut Value, path: &[&str], value: Value) {
    if path.is_empty() {
        return;
    }
    let mut node = root;
    for seg in &path[..path.len() - 1] {
        if !node.is_object() {
            *node = Value::Object(Map::new());
        }
        let Some(obj) = node.as_object_mut() else {
            return;
        };
        node = obj
            .entry((*seg).to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    if !node.is_object() {
        *node = Value::Object(Map::new());
    }
    let Some(obj) = node.as_object_mut() else {
        return;
    };
    obj.insert(path[path.len() - 1].to_owned(), value);
}
