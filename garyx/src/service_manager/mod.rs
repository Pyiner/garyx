//! Cross-platform gateway service management.
//!
//! Exposes a [`ServiceManager`] trait that hides the platform-specific
//! mechanics of installing, starting, stopping, and removing the managed
//! garyx gateway. At runtime we pick the right implementation (launchd on
//! macOS, systemd user services on Linux) via [`active_manager`].
//!
//! Callers should build a [`ServiceSpec`], grab the active manager, then
//! invoke trait methods — they never need to branch on target_os themselves.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(target_os = "macos")]
pub(crate) mod launchd;
#[cfg(target_os = "linux")]
pub(crate) mod systemd;

/// Declarative description of the gateway service a platform backend should
/// install and supervise.
#[derive(Debug, Clone)]
pub(crate) struct ServiceSpec {
    /// Absolute path to the garyx binary the service should execute.
    pub binary_path: PathBuf,
    /// Interface the gateway should bind to (e.g. "0.0.0.0").
    pub host: String,
    /// TCP port for the gateway.
    pub port: u16,
    /// Directory for stdout/stderr log files. Backends create it if missing.
    pub log_dir: PathBuf,
    /// Workspace root (a garyx checkout) exposed to the service as
    /// `GARYX_WORKSPACE_ROOT`. Used by the in-process rebuild+restart flow
    /// during development.
    pub workspace_root: Option<PathBuf>,
}

/// Result of an install / restart operation — the caller uses this to print a
/// friendly confirmation plus any setup hints (for example, a reminder to
/// enable linger on Linux).
#[derive(Debug, Clone)]
pub(crate) struct InstallReport {
    /// Absolute path of the unit file / plist that was written.
    pub unit_path: PathBuf,
    /// Short human-readable backend name, e.g. "launchd" or "systemd (--user)".
    pub backend: &'static str,
    /// Optional post-install warnings / hints the CLI should echo.
    pub warnings: Vec<String>,
}

pub(crate) trait ServiceManager {
    /// Display name of this backend (e.g. "launchd", "systemd (--user)").
    fn backend_name(&self) -> &'static str;

    /// Write the unit / plist to disk, register it with the platform
    /// supervisor, and start the service. Idempotent — re-running refreshes
    /// the unit file and restarts the service.
    fn install(&self, spec: &ServiceSpec) -> Result<InstallReport, Box<dyn std::error::Error>>;

    /// Stop the service if running and remove the unit / plist file.
    fn uninstall(&self) -> Result<(), Box<dyn std::error::Error>>;

    /// Start an already-installed service. Errors if the unit file is missing.
    fn start(&self) -> Result<(), Box<dyn std::error::Error>>;

    /// Stop the service. No-op if it's already stopped.
    fn stop(&self) -> Result<(), Box<dyn std::error::Error>>;

    /// Refresh the unit file from `spec` and restart the service. Works whether
    /// or not the service was installed previously.
    fn restart(&self, spec: &ServiceSpec) -> Result<InstallReport, Box<dyn std::error::Error>>;

    /// Whether the unit / plist is currently present on disk.
    fn is_installed(&self) -> bool;
}

/// Pick the right [`ServiceManager`] for this host.
#[cfg(target_os = "macos")]
pub(crate) fn active_manager() -> Result<Box<dyn ServiceManager>, Box<dyn std::error::Error>> {
    Ok(Box::new(launchd::LaunchdManager::new()))
}

#[cfg(target_os = "linux")]
pub(crate) fn active_manager() -> Result<Box<dyn ServiceManager>, Box<dyn std::error::Error>> {
    Ok(Box::new(systemd::SystemdUserManager::new()))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn active_manager() -> Result<Box<dyn ServiceManager>, Box<dyn std::error::Error>> {
    Err(format!(
        "`garyx gateway install/start/stop/restart/uninstall` is not supported on {} yet — run `garyx gateway run` directly and supervise it yourself.",
        std::env::consts::OS
    )
    .into())
}

/// Absolute path to the per-user log directory (`$HOME/.garyx/logs`).
pub(crate) fn log_dir_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let home = std::env::var("HOME").map_err(|_| "HOME is not set")?;
    Ok(PathBuf::from(home).join(".garyx").join("logs"))
}

/// Quick TCP probe — returns true if something is accepting connections on
/// the given port on loopback. This replaces the old `lsof`-based probe so
/// we don't depend on a macOS-only tool path.
pub(crate) fn port_is_open(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

/// Quote a path so it can be embedded as one argument inside a shell command
/// that is itself wrapped in double quotes.
pub(crate) fn shell_double_quoted_arg_for_nested_command(path: &Path) -> String {
    let mut escaped = String::new();
    for ch in path.display().to_string().chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '$' => escaped.push_str("\\$"),
            '`' => escaped.push_str("\\`"),
            _ => escaped.push(ch),
        }
    }
    format!(r#"\"{escaped}\""#)
}

/// Poll `port_is_open` until it returns true or the deadline expires.
pub(crate) async fn wait_for_port(
    port: u16,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if port_is_open(port) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Err(format!("gateway did not start listening on port {port} in time").into())
}

/// Ensure a parent directory exists for the given path.
#[allow(dead_code)]
pub(crate) fn ensure_parent_dir(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}
