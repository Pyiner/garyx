use super::*;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::GaryxConfig;
use serde_json::Value;
use tempfile::TempDir;

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

fn clear_gateway_env_overrides() {
    unsafe {
        std::env::remove_var("GARYX_GATEWAY_HOST");
        std::env::remove_var("GARYX_GATEWAY_PORT");
    }
}

struct HomeEnvGuard {
    previous_home: Option<std::ffi::OsString>,
    previous_userprofile: Option<std::ffi::OsString>,
}

impl HomeEnvGuard {
    fn set(path: &Path) -> Self {
        let guard = Self {
            previous_home: std::env::var_os("HOME"),
            previous_userprofile: std::env::var_os("USERPROFILE"),
        };
        unsafe {
            std::env::set_var("HOME", path);
            std::env::remove_var("USERPROFILE");
        }
        guard
    }
}

impl Drop for HomeEnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.previous_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match &self.previous_userprofile {
                Some(value) => std::env::set_var("USERPROFILE", value),
                None => std::env::remove_var("USERPROFILE"),
            }
        }
    }
}

#[test]
fn load_merges_defaults_and_substitutes_env() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    unsafe {
        std::env::set_var("GARYX_TEST_HOST", "127.0.0.1");
    }
    write_json(
        &config_path,
        &serde_json::json!({
            "gateway": { "host": "${GARYX_TEST_HOST}" },
            "sessions": { "data_dir": "./data" }
        }),
    );

    let loaded = load_config(&config_path, &ConfigLoadOptions::default()).unwrap();
    assert_eq!(loaded.config.gateway.host, "127.0.0.1");
    assert_eq!(loaded.config.gateway.port, 31337);
    assert!(loaded.config.sessions.data_dir.unwrap().contains("data"));

    unsafe {
        std::env::remove_var("GARYX_TEST_HOST");
    }
}

#[test]
fn default_load_path_uses_home_hidden_garyx_json() {
    let opts = ConfigLoadOptions::default();
    let path = opts.default_path.to_string_lossy();
    assert!(
        path.ends_with(".garyx/garyx.json") || path.ends_with(".garyx\\garyx.json"),
        "expected path ending in .garyx/garyx.json, got: {path}"
    );
}

#[test]
fn load_migrates_legacy_default_config_to_hidden_garyx_dir() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();
    let _home = HomeEnvGuard::set(tmp.path());

    // Place config at legacy path ~/.gary/gary.json
    let legacy_dir = tmp.path().join(".gary");
    let current_dir = tmp.path().join(".garyx");
    fs::create_dir_all(&legacy_dir).unwrap();
    let legacy_path = legacy_dir.join("gary.json");
    write_json(
        &legacy_path,
        &serde_json::json!({
            "gateway": { "port": 4242 }
        }),
    );

    let loaded = load_config("", &ConfigLoadOptions::default()).unwrap();

    assert_eq!(loaded.path, current_dir.join("garyx.json"));
    assert_eq!(loaded.config.gateway.port, 4242);
    assert!(loaded.path.exists());
    assert!(!legacy_path.exists());
    assert!(
        loaded
            .diagnostics
            .warnings
            .iter()
            .any(|warning| warning.code == "CONFIG_PATH_MIGRATED")
    );
}

#[test]
fn load_falls_back_to_legacy_path_when_hidden_garyx_dir_cannot_be_created() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();
    let _home = HomeEnvGuard::set(tmp.path());

    // Place config at legacy path ~/.gary/gary.json
    let legacy_dir = tmp.path().join(".gary");
    fs::create_dir_all(&legacy_dir).unwrap();
    let legacy_path = legacy_dir.join("gary.json");
    write_json(
        &legacy_path,
        &serde_json::json!({
            "gateway": { "host": "127.0.0.1" }
        }),
    );

    // Block creation of ~/.garyx by placing a file there
    fs::write(tmp.path().join(".garyx"), b"not-a-directory").unwrap();

    let loaded = load_config("", &ConfigLoadOptions::default()).unwrap();

    assert_eq!(loaded.path, legacy_path);
    assert_eq!(loaded.config.gateway.host, "127.0.0.1");
    assert!(loaded.path.exists());
    assert!(
        loaded
            .diagnostics
            .warnings
            .iter()
            .any(|warning| warning.code == "CONFIG_PATH_LEGACY_FALLBACK")
    );
}

#[test]
fn load_fails_when_required_env_missing() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    write_json(
        &config_path,
        &serde_json::json!({
            "gateway": { "host": "${GARYX_TEST_MISSING}" }
        }),
    );

    let err = load_config(&config_path, &ConfigLoadOptions::default()).unwrap_err();
    assert!(
        err.diagnostics
            .errors
            .iter()
            .any(|d| d.code == "CONFIG_ENV_MISSING")
    );
}

#[test]
fn load_applies_runtime_overrides() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");
    write_json(&config_path, &serde_json::json!({}));

    unsafe {
        std::env::set_var("GARYX_GATEWAY_PORT", "9090");
    }

    let mut opts = ConfigLoadOptions::default();
    opts.runtime_overrides.gateway_host = Some("0.0.0.0".to_owned());

    let loaded = load_config(&config_path, &opts).unwrap();
    assert_eq!(loaded.config.gateway.host, "0.0.0.0");
    assert_eq!(loaded.config.gateway.port, 9090);

    unsafe {
        std::env::remove_var("GARYX_GATEWAY_PORT");
    }
}

#[test]
fn load_trims_gateway_host_env_override() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");
    write_json(&config_path, &serde_json::json!({}));

    unsafe {
        std::env::set_var("GARYX_GATEWAY_HOST", "  0.0.0.0  ");
    }

    let loaded = load_config(&config_path, &ConfigLoadOptions::default()).unwrap();
    assert_eq!(loaded.config.gateway.host, "0.0.0.0");

    unsafe {
        std::env::remove_var("GARYX_GATEWAY_HOST");
    }
}

#[test]
fn load_trims_gateway_port_env_override() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");
    write_json(&config_path, &serde_json::json!({}));

    unsafe {
        std::env::set_var("GARYX_GATEWAY_PORT", "  31337  ");
    }

    let loaded = load_config(&config_path, &ConfigLoadOptions::default()).unwrap();
    assert_eq!(loaded.config.gateway.port, 31337);

    unsafe {
        std::env::remove_var("GARYX_GATEWAY_PORT");
    }
}

#[test]
fn load_ignores_legacy_agent_defaults_workspace_dir_with_warning() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");
    write_json(
        &config_path,
        &serde_json::json!({
            "agent_defaults": {
                "workspace_dir": "~/gary",
                "heartbeat": {
                    "enabled": false
                }
            }
        }),
    );

    let loaded = load_config(&config_path, &ConfigLoadOptions::default()).unwrap();
    let serialized = serde_json::to_value(&loaded.config).unwrap();

    assert!(serialized["agent_defaults"].get("workspace_dir").is_none());
    assert!(
        loaded
            .diagnostics
            .warnings
            .iter()
            .any(|warning| warning.code == "CONFIG_DEPRECATED_FIELD_IGNORED"
                && warning.path.as_deref() == Some("$.agent_defaults.workspace_dir"))
    );
}

#[test]
fn load_ignores_legacy_sessions_fields_with_warning() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");
    write_json(
        &config_path,
        &serde_json::json!({
            "sessions": {
                "redis": {
                    "host": "localhost",
                    "port": 6379
                },
                "store_type": "memory"
            }
        }),
    );

    let loaded = load_config(&config_path, &ConfigLoadOptions::default()).unwrap();
    let serialized = serde_json::to_value(&loaded.config).unwrap();

    assert!(serialized["sessions"].get("redis").is_none());
    assert!(serialized["sessions"].get("store_type").is_none());
    assert!(
        loaded
            .diagnostics
            .warnings
            .iter()
            .any(|warning| warning.code == "CONFIG_DEPRECATED_FIELD_IGNORED"
                && warning.path.as_deref() == Some("$.sessions.redis"))
    );
    assert!(
        loaded
            .diagnostics
            .warnings
            .iter()
            .any(|warning| warning.code == "CONFIG_DEPRECATED_FIELD_IGNORED"
                && warning.path.as_deref() == Some("$.sessions.store_type"))
    );
}

#[test]
fn write_config_atomic_creates_backups() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");
    fs::write(&config_path, "{}").unwrap();

    let mut cfg = GaryxConfig::default();
    cfg.gateway.port = 4001;

    let opts = ConfigWriteOptions {
        backup_keep: 2,
        mode: None,
    };
    write_config_atomic(&config_path, &cfg, &opts).unwrap();
    let backups = list_backups(&config_path).unwrap();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        backups[0]
            .parent()
            .and_then(|path| path.file_name())
            .unwrap(),
        std::ffi::OsStr::new("backups")
    );

    cfg.gateway.port = 4002;
    write_config_atomic(&config_path, &cfg, &opts).unwrap();
    let backups = list_backups(&config_path).unwrap();
    assert_eq!(backups.len(), 2);
    assert!(backups.iter().all(|path| {
        path.parent().and_then(|parent| parent.file_name()) == Some(std::ffi::OsStr::new("backups"))
    }));
}

#[test]
fn write_config_value_atomic_strips_legacy_agent_defaults_workspace_dir() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    write_config_value_atomic(
        &config_path,
        &serde_json::json!({
            "agent_defaults": {
                "workspace_dir": "~/gary",
                "heartbeat": {
                    "enabled": false
                }
            },
            "gateway": {
                "port": 31337
            }
        }),
        &ConfigWriteOptions::default(),
    )
    .unwrap();

    let persisted =
        serde_json::from_str::<Value>(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert!(persisted["agent_defaults"].get("workspace_dir").is_none());
}

#[test]
fn write_config_value_atomic_strips_legacy_sessions_fields() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    write_config_value_atomic(
        &config_path,
        &serde_json::json!({
            "sessions": {
                "redis": {
                    "host": "localhost",
                    "port": 6379
                },
                "store_type": "memory"
            },
            "gateway": {
                "port": 31337
            }
        }),
        &ConfigWriteOptions::default(),
    )
    .unwrap();

    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    assert!(saved.get("sessions").is_none());
}

#[test]
fn hot_reload_keeps_last_good_on_invalid_update() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");
    write_json(
        &config_path,
        &serde_json::json!({
            "gateway": { "host": "127.0.0.1" }
        }),
    );

    let initial = load_config(&config_path, &ConfigLoadOptions::default()).unwrap();
    let opts = ConfigHotReloadOptions {
        poll_interval: Duration::from_millis(40),
        debounce: Duration::from_millis(80),
        load_options: ConfigLoadOptions {
            default_path: config_path.clone(),
            runtime_overrides: ConfigRuntimeOverrides::default(),
        },
    };
    let reloader = ConfigHotReloader::start(config_path.clone(), initial.config, opts);

    let seen_hosts: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_hosts_cb = seen_hosts.clone();
    reloader.register_callback(move |cfg, _| {
        seen_hosts_cb.lock().unwrap().push(cfg.gateway.host);
    });

    // Ensure file mtime differs from initial write on filesystems with coarse
    // timestamp resolution.
    thread::sleep(Duration::from_millis(1100));
    write_json(
        &config_path,
        &serde_json::json!({
            "gateway": { "host": "0.0.0.0" }
        }),
    );
    let deadline = Instant::now() + Duration::from_millis(1200);
    while Instant::now() < deadline {
        if seen_hosts
            .lock()
            .unwrap()
            .iter()
            .any(|host| host == "0.0.0.0")
        {
            break;
        }
        thread::sleep(Duration::from_millis(40));
    }

    fs::write(&config_path, "[1,2,3]").unwrap();
    thread::sleep(Duration::from_millis(280));

    reloader.stop();
    let hosts = seen_hosts.lock().unwrap().clone();
    assert!(
        hosts.iter().any(|h| h == "0.0.0.0"),
        "hosts={hosts:?} metrics={:?}",
        reloader.metrics()
    );
    assert_eq!(hosts.iter().filter(|h| *h == "0.0.0.0").count(), 1);
    assert!(reloader.metrics().failures >= 1);
}

// -----------------------------------------------------------------------
// Env var substitution tests
// -----------------------------------------------------------------------

#[test]
fn env_substitution_with_default_value() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    // Use :- syntax with a default value; env var not set.
    write_json(
        &config_path,
        &serde_json::json!({
            "gateway": { "host": "${GARYX_TEST_UNSET_WITH_DEFAULT:-localhost}" }
        }),
    );

    let loaded = load_config(
        &config_path,
        &ConfigLoadOptions {
            default_path: config_path.clone(),
            runtime_overrides: ConfigRuntimeOverrides {
                gateway_port: Some(8080),
                gateway_host: None,
            },
        },
    )
    .unwrap();
    assert_eq!(loaded.config.gateway.host, "localhost");
}

#[test]
fn env_substitution_set_var_overrides_default() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    unsafe {
        std::env::set_var("GARYX_TEST_SET_WITH_DEFAULT", "10.0.0.1");
    }

    write_json(
        &config_path,
        &serde_json::json!({
            "gateway": { "host": "${GARYX_TEST_SET_WITH_DEFAULT:-fallback}" }
        }),
    );

    let loaded = load_config(&config_path, &ConfigLoadOptions::default()).unwrap();
    assert_eq!(loaded.config.gateway.host, "10.0.0.1");

    unsafe {
        std::env::remove_var("GARYX_TEST_SET_WITH_DEFAULT");
    }
}

#[test]
fn env_substitution_empty_default() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    write_json(
        &config_path,
        &serde_json::json!({
            "gateway": { "public_url": "${GARYX_TEST_EMPTY_DEFAULT:-}" }
        }),
    );

    let loaded = load_config(&config_path, &ConfigLoadOptions::default()).unwrap();
    assert_eq!(loaded.config.gateway.public_url, "");
}

#[test]
fn env_substitution_no_op_when_no_patterns() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    write_json(
        &config_path,
        &serde_json::json!({
            "gateway": { "host": "plain-host", "public_url": "https://example.com" }
        }),
    );

    let loaded = load_config(&config_path, &ConfigLoadOptions::default()).unwrap();
    assert_eq!(loaded.config.gateway.host, "plain-host");
    assert_eq!(loaded.config.gateway.public_url, "https://example.com");
}

// -----------------------------------------------------------------------
// $include tests
// -----------------------------------------------------------------------

#[test]
fn include_simple_merge() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();

    let partial_path = tmp.path().join("gateway.json");
    write_json(
        &partial_path,
        &serde_json::json!({
            "host": "included-host",
            "port": 8080
        }),
    );

    let config_path = tmp.path().join("gary.json");
    write_json(
        &config_path,
        &serde_json::json!({
            "gateway": {
                "$include": "gateway.json",
                "host": "override-host"
            }
        }),
    );

    let loaded = load_config(&config_path, &ConfigLoadOptions::default()).unwrap();
    // Existing key takes precedence over included.
    assert_eq!(loaded.config.gateway.host, "override-host");
    // Included key fills in missing.
    assert_eq!(loaded.config.gateway.port, 8080);
}

#[test]
fn include_nested() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();

    // inner.json
    let inner = tmp.path().join("inner.json");
    write_json(
        &inner,
        &serde_json::json!({
            "host": "from-inner",
            "port": 7777
        }),
    );

    // outer.json includes inner.json
    let outer = tmp.path().join("outer.json");
    write_json(
        &outer,
        &serde_json::json!({
            "$include": "inner.json",
            "port": 9999
        }),
    );

    // main config includes outer.json under gateway
    let config_path = tmp.path().join("gary.json");
    write_json(
        &config_path,
        &serde_json::json!({
            "gateway": {
                "$include": "outer.json"
            }
        }),
    );

    let loaded = load_config(&config_path, &ConfigLoadOptions::default()).unwrap();
    assert_eq!(loaded.config.gateway.host, "from-inner");
    // outer.json overrides inner.json's port
    assert_eq!(loaded.config.gateway.port, 9999);
}

#[test]
fn include_circular_detected() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();

    let a = tmp.path().join("a.json");
    let b = tmp.path().join("b.json");

    write_json(
        &a,
        &serde_json::json!({
            "$include": "b.json",
            "host": "from-a"
        }),
    );
    write_json(
        &b,
        &serde_json::json!({
            "$include": "a.json",
            "port": 1234
        }),
    );

    let config_path = tmp.path().join("gary.json");
    write_json(
        &config_path,
        &serde_json::json!({
            "gateway": {
                "$include": "a.json"
            }
        }),
    );

    let err = load_config(&config_path, &ConfigLoadOptions::default()).unwrap_err();
    assert!(
        err.diagnostics
            .errors
            .iter()
            .any(|d| d.code == "CONFIG_INCLUDE_CIRCULAR"),
        "expected circular include error, got: {:?}",
        err.diagnostics.errors
    );
}

#[test]
fn include_missing_file() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    write_json(
        &config_path,
        &serde_json::json!({
            "gateway": {
                "$include": "nonexistent.json"
            }
        }),
    );

    let err = load_config(&config_path, &ConfigLoadOptions::default()).unwrap_err();
    assert!(
        err.diagnostics
            .errors
            .iter()
            .any(|d| d.code == "CONFIG_INCLUDE_IO")
    );
}

#[test]
fn include_with_env_substitution() {
    let _env = env_lock();
    clear_gateway_env_overrides();
    let tmp = TempDir::new().unwrap();

    unsafe {
        std::env::set_var("GARYX_TEST_INC_HOST", "env-host");
    }

    let partial = tmp.path().join("partial.json");
    write_json(
        &partial,
        &serde_json::json!({
            "host": "${GARYX_TEST_INC_HOST}"
        }),
    );

    let config_path = tmp.path().join("gary.json");
    write_json(
        &config_path,
        &serde_json::json!({
            "gateway": {
                "$include": "partial.json"
            }
        }),
    );

    let loaded = load_config(&config_path, &ConfigLoadOptions::default()).unwrap();
    assert_eq!(loaded.config.gateway.host, "env-host");

    unsafe {
        std::env::remove_var("GARYX_TEST_INC_HOST");
    }
}

// -----------------------------------------------------------------------
// Backup / restore tests
// -----------------------------------------------------------------------

#[test]
fn backup_config_creates_timestamped_file() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");
    fs::write(&config_path, r#"{"gateway":{}}"#).unwrap();

    let backup = backup_config(&config_path).unwrap();
    assert!(backup.is_some());
    let backup_path = backup.unwrap();
    assert!(backup_path.exists());
    assert_eq!(
        backup_path
            .parent()
            .and_then(|path| path.file_name())
            .unwrap(),
        std::ffi::OsStr::new("backups")
    );

    let backup_name = backup_path.file_name().unwrap().to_string_lossy();
    assert!(backup_name.starts_with("gary.json.backup."));
}

#[test]
fn backup_config_nonexistent_returns_none() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("no-such.json");

    let result = backup_config(&config_path).unwrap();
    assert!(result.is_none());
}

#[test]
fn list_backups_finds_both_styles() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");
    fs::write(&config_path, "{}").unwrap();

    // Create timestamped backup.
    fs::create_dir_all(tmp.path().join("backups")).unwrap();
    fs::write(
        tmp.path()
            .join("backups")
            .join("gary.json.backup.1700000000"),
        "backup-ts",
    )
    .unwrap();
    // Create rotate-style backup.
    fs::write(tmp.path().join("gary.json.bak.1"), "backup-rotate").unwrap();

    let backups = list_backups(&config_path).unwrap();
    assert_eq!(backups.len(), 2);
}

#[test]
fn restore_config_cycle() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");
    fs::write(&config_path, r#"{"version":"original"}"#).unwrap();

    // Manually create a backup (don't use backup_config to avoid timestamp
    // collision with the backup created inside restore_config).
    fs::create_dir_all(tmp.path().join("backups")).unwrap();
    let backup = tmp.path().join("backups").join("gary.json.backup.manual");
    fs::copy(&config_path, &backup).unwrap();

    // Overwrite config.
    fs::write(&config_path, r#"{"version":"modified"}"#).unwrap();

    // Restore from backup — this also backs up the current "modified" version.
    restore_config(&backup, &config_path).unwrap();

    let restored = fs::read_to_string(&config_path).unwrap();
    assert!(restored.contains("original"));
}

#[test]
fn restore_nonexistent_backup_fails() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");
    let missing_backup = tmp.path().join("no-such-backup.json");

    let result = restore_config(&missing_backup, &config_path);
    assert!(result.is_err());
}
