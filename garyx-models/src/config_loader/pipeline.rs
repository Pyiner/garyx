use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Map, Value};

use super::diagnostics::ConfigDiagnostics;

pub(super) fn resolve_config_path(input: &Path, default_path: &Path) -> PathBuf {
    if input.as_os_str().is_empty() {
        return default_path.to_path_buf();
    }
    input.to_path_buf()
}

fn env_pattern() -> &'static Regex {
    static ENV_RE: OnceLock<Regex> = OnceLock::new();
    ENV_RE.get_or_init(|| {
        Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-(.*?))?\}")
            .expect("valid env substitution regex")
    })
}

pub(super) fn substitute_env_in_value(
    value: &mut Value,
    json_path: &str,
    diagnostics: &mut ConfigDiagnostics,
) {
    match value {
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                let child_path = format!("{json_path}.{k}");
                substitute_env_in_value(v, &child_path, diagnostics);
            }
        }
        Value::Array(arr) => {
            for (idx, v) in arr.iter_mut().enumerate() {
                let child_path = format!("{json_path}[{idx}]");
                substitute_env_in_value(v, &child_path, diagnostics);
            }
        }
        Value::String(s) => {
            let replaced = env_pattern()
                .replace_all(s, |caps: &regex::Captures<'_>| {
                    let var_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
                    let default = caps.get(2).map(|m| m.as_str());
                    match std::env::var(var_name) {
                        Ok(v) => v,
                        Err(_) => {
                            if let Some(def) = default {
                                def.to_owned()
                            } else {
                                diagnostics.push_error(
                                    "CONFIG_ENV_MISSING",
                                    format!("missing required environment variable: {var_name}"),
                                    Some(json_path.to_owned()),
                                );
                                String::new()
                            }
                        }
                    }
                })
                .to_string();
            *s = replaced;
        }
        _ => {}
    }
}

pub(super) fn merge_defaults(defaults: &Value, input: &Value) -> Value {
    match (defaults, input) {
        (Value::Object(def_map), Value::Object(in_map)) => {
            let mut merged: Map<String, Value> = def_map.clone();
            for (k, in_val) in in_map {
                let next = if let Some(def_val) = def_map.get(k) {
                    merge_defaults(def_val, in_val)
                } else {
                    in_val.clone()
                };
                merged.insert(k.clone(), next);
            }
            Value::Object(merged)
        }
        (_, v) => v.clone(),
    }
}

pub fn strip_legacy_config_fields(
    value: &mut Value,
    mut diagnostics: Option<&mut ConfigDiagnostics>,
) {
    let Some(root) = value.as_object_mut() else {
        return;
    };

    if root.remove("agent_defaults").is_some() {
        if let Some(diagnostics) = diagnostics.as_deref_mut() {
            diagnostics.push_warning(
                "CONFIG_DEPRECATED_FIELD_IGNORED",
                "agent_defaults is deprecated and ignored",
                Some("$.agent_defaults"),
            );
        }
    }

    let remove_sessions = if let Some(sessions) =
        root.get_mut("sessions").and_then(Value::as_object_mut)
    {
        if sessions.remove("redis").is_some() {
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.push_warning(
                    "CONFIG_DEPRECATED_FIELD_IGNORED",
                    "sessions.redis is deprecated and ignored; Garyx now persists thread history on local disk only",
                    Some("$.sessions.redis"),
                );
            }
        }
        if sessions.remove("store_type").is_some() {
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.push_warning(
                    "CONFIG_DEPRECATED_FIELD_IGNORED",
                    "sessions.store_type is deprecated and ignored; Garyx now persists thread history on local disk only",
                    Some("$.sessions.store_type"),
                );
            }
        }
        sessions.is_empty()
    } else {
        false
    };

    if remove_sessions {
        root.remove("sessions");
    }

    flatten_legacy_plugin_channels(value, diagnostics);
}

pub fn strip_redundant_config_fields(value: &mut Value) {
    strip_legacy_config_fields(value, None);
    strip_redundant_channel_account_fields(value);
}

fn flatten_legacy_plugin_channels(
    value: &mut Value,
    mut diagnostics: Option<&mut ConfigDiagnostics>,
) {
    let Some(channels) = value
        .as_object_mut()
        .and_then(|root| root.get_mut("channels"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };

    let Some(legacy_plugins) = channels.remove("plugins") else {
        return;
    };

    let Some(legacy_plugins) = legacy_plugins.as_object() else {
        if let Some(diagnostics) = diagnostics.as_deref_mut() {
            diagnostics.push_warning(
                "CONFIG_LEGACY_CHANNEL_PLUGINS_INVALID",
                "channels.plugins must be an object; ignoring legacy plugin channel bucket",
                Some("$.channels.plugins"),
            );
        }
        return;
    };

    for (channel_id, channel_cfg) in legacy_plugins {
        if channels.contains_key(channel_id) {
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.push_warning(
                    "CONFIG_LEGACY_CHANNEL_PLUGINS_CONFLICT",
                    format!(
                        "channels.{channel_id} overrides legacy channels.plugins.{channel_id}; ignoring legacy entry"
                    ),
                    Some(format!("$.channels.plugins.{channel_id}")),
                );
            }
            continue;
        }
        channels.insert(channel_id.clone(), channel_cfg.clone());
    }
}

fn strip_redundant_channel_account_fields(root: &mut Value) {
    for account_path in [&["channels", "api", "accounts"][..]] {
        let Some(accounts) = get_mut_value(root, account_path).and_then(Value::as_object_mut)
        else {
            continue;
        };

        for account in accounts.values_mut() {
            let Some(account) = account.as_object_mut() else {
                continue;
            };
            sanitize_channel_account(account);
        }
    }

    let Some(channels) = get_mut_value(root, &["channels"]).and_then(Value::as_object_mut) else {
        return;
    };
    for (channel_id, channel_cfg) in channels.iter_mut() {
        if channel_id == "api" {
            continue;
        }
        let Some(accounts) = channel_cfg
            .get_mut("accounts")
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        for account in accounts.values_mut() {
            let Some(account) = account.as_object_mut() else {
                continue;
            };
            sanitize_channel_account(account);
        }
    }
}

fn sanitize_channel_account(account: &mut Map<String, Value>) {
    remove_null_or_blank_string(account, "name");
    remove_null_or_blank_string(account, "workspace_dir");
    remove_blank_string(account, "agent_id");
}

fn remove_blank_string(map: &mut Map<String, Value>, key: &str) {
    let should_remove = matches!(map.get(key), Some(Value::String(value)) if value.trim().is_empty())
        || matches!(map.get(key), Some(Value::Null));
    if should_remove {
        map.remove(key);
    }
}

fn remove_null_or_blank_string(map: &mut Map<String, Value>, key: &str) {
    remove_blank_string(map, key);
}

pub(super) fn normalize_paths(
    value: &mut Value,
    base_dir: &Path,
    diagnostics: &mut ConfigDiagnostics,
) {
    normalize_string_path(value, &["sessions", "data_dir"], base_dir, diagnostics);

    normalize_account_workspace_paths(
        value,
        &["channels", "api", "accounts"],
        base_dir,
        diagnostics,
    );
    normalize_channel_account_workspace_paths(value, base_dir, diagnostics);
}

fn normalize_account_workspace_paths(
    root: &mut Value,
    account_path: &[&str],
    base_dir: &Path,
    diagnostics: &mut ConfigDiagnostics,
) {
    let mut node = root;
    for seg in account_path {
        node = match node.get_mut(*seg) {
            Some(n) => n,
            None => return,
        };
    }
    let Some(accounts) = node.as_object_mut() else {
        return;
    };

    for (account_id, account_val) in accounts.iter_mut() {
        if let Some(ws) = account_val.get_mut("workspace_dir") {
            if let Some(path_str) = ws.as_str() {
                let normalized = normalize_one_path(
                    path_str,
                    base_dir,
                    diagnostics,
                    &format!("$.{}.{}.workspace_dir", account_path.join("."), account_id),
                );
                *ws = Value::String(normalized);
            }
        }
    }
}

fn normalize_channel_account_workspace_paths(
    root: &mut Value,
    base_dir: &Path,
    diagnostics: &mut ConfigDiagnostics,
) {
    let Some(channels) = get_mut_value(root, &["channels"]).and_then(Value::as_object_mut) else {
        return;
    };
    for (channel_id, channel_cfg) in channels.iter_mut() {
        if channel_id == "api" {
            continue;
        }
        let Some(accounts) = channel_cfg
            .get_mut("accounts")
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        for (account_id, account_val) in accounts.iter_mut() {
            if let Some(ws) = account_val.get_mut("workspace_dir") {
                if let Some(path_str) = ws.as_str() {
                    let normalized = normalize_one_path(
                        path_str,
                        base_dir,
                        diagnostics,
                        &format!("$.channels.{channel_id}.accounts.{account_id}.workspace_dir"),
                    );
                    *ws = Value::String(normalized);
                }
            }
        }
    }
}

fn normalize_string_path(
    root: &mut Value,
    key_path: &[&str],
    base_dir: &Path,
    diagnostics: &mut ConfigDiagnostics,
) {
    let Some(target) = get_mut_value(root, key_path) else {
        return;
    };
    let Some(path_str) = target.as_str() else {
        return;
    };
    let normalized = normalize_one_path(
        path_str,
        base_dir,
        diagnostics,
        &format!("$.{}", key_path.join(".")),
    );
    *target = Value::String(normalized);
}

fn normalize_one_path(
    raw: &str,
    base_dir: &Path,
    diagnostics: &mut ConfigDiagnostics,
    json_path: &str,
) -> String {
    let mut path = expand_home(raw).unwrap_or_else(|| PathBuf::from(raw));
    if path.is_relative() {
        path = base_dir.join(path);
    }

    if path.exists() {
        match fs::canonicalize(&path) {
            Ok(c) => path = c,
            Err(e) => diagnostics.push_warning(
                "CONFIG_PATH_CANONICALIZE",
                format!("failed to canonicalize path {}: {e}", path.display()),
                Some(json_path.to_owned()),
            ),
        }
    }

    path.to_string_lossy().to_string()
}

fn expand_home(raw: &str) -> Option<PathBuf> {
    if raw == "~" {
        return home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home_dir().map(|h| h.join(rest));
    }
    None
}

fn home_dir() -> Option<PathBuf> {
    crate::local_paths::home_dir()
}

fn get_mut_value<'a>(root: &'a mut Value, path: &[&str]) -> Option<&'a mut Value> {
    let mut node = root;
    for seg in path {
        node = node.get_mut(*seg)?;
    }
    Some(node)
}
