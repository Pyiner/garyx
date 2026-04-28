//! Runtime restart helpers.
//!
//! Provides best-effort restart orchestration shared by `/api/restart` and MCP
//! restart tool handlers.

#[cfg(not(test))]
use std::fs;
#[cfg(not(test))]
use std::path::PathBuf;
#[cfg(not(test))]
use std::process::Stdio;
#[cfg(not(test))]
use std::time::Duration;

#[cfg(not(test))]
use garyx_models::local_paths::default_pending_restart_path;
#[cfg(not(test))]
use serde::{Deserialize, Serialize};
#[cfg(not(test))]
use tokio::process::Command;

#[cfg(not(test))]
const LAUNCHD_SERVICE_NAME: &str = "com.garyx.agent";

#[derive(Debug, Clone)]
pub struct RestartOptions {
    pub reason: String,
    pub build_before_restart: bool,
    pub continue_thread_id: Option<String>,
    pub continue_run_id: Option<String>,
}

impl RestartOptions {
    pub fn new(reason: String) -> Self {
        Self {
            reason,
            build_before_restart: false,
            continue_thread_id: None,
            continue_run_id: None,
        }
    }
}

const DEFAULT_RESTART_CONTINUATION_MESSAGE: &str = concat!(
    "<system-restart>",
    "The gateway restarted while handling the active task. ",
    "This is a system continuation notice, not a new user request. ",
    "Continue from the previous task state and keep responding in the user's language.",
    "</system-restart>"
);

pub(crate) fn default_restart_continuation_message() -> &'static str {
    DEFAULT_RESTART_CONTINUATION_MESSAGE
}

#[cfg(not(test))]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingRestartContinuation {
    thread_id: String,
    message: String,
    reason: String,
    run_id: Option<String>,
    created_at_epoch_ms: u64,
}

#[cfg(not(test))]
fn pending_restart_file_path() -> Option<PathBuf> {
    let _ = std::env::var_os("HOME")?;
    Some(default_pending_restart_path())
}

#[cfg(not(test))]
fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(not(test))]
pub(crate) fn read_pending_continuations() -> Result<Vec<serde_json::Value>, String> {
    let Some(path) = pending_restart_file_path() else {
        return Ok(Vec::new());
    };
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("read {} failed: {error}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str::<Vec<serde_json::Value>>(&raw)
        .map_err(|error| format!("parse {} failed: {error}", path.display()))
}

#[cfg(test)]
pub(crate) fn read_pending_continuations() -> Result<Vec<serde_json::Value>, String> {
    Ok(Vec::new())
}

#[cfg(not(test))]
pub(crate) fn clear_pending_continuations() -> Result<(), String> {
    let Some(path) = pending_restart_file_path() else {
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(&path).map_err(|error| format!("remove {} failed: {error}", path.display()))
}

#[cfg(test)]
pub(crate) fn clear_pending_continuations() -> Result<(), String> {
    Ok(())
}

#[cfg(not(test))]
pub(crate) fn write_pending_continuations(entries: &[serde_json::Value]) -> Result<(), String> {
    let Some(path) = pending_restart_file_path() else {
        return Ok(());
    };
    if entries.is_empty() {
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|error| format!("remove {} failed: {error}", path.display()))?;
        }
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {} failed: {error}", parent.display()))?;
    }
    let encoded = serde_json::to_vec_pretty(entries)
        .map_err(|error| format!("serialize {} failed: {error}", path.display()))?;
    fs::write(&path, encoded).map_err(|error| format!("write {} failed: {error}", path.display()))
}

#[cfg(test)]
pub(crate) fn write_pending_continuations(_entries: &[serde_json::Value]) -> Result<(), String> {
    Ok(())
}

#[cfg(not(test))]
fn append_pending_continuation(entry: PendingRestartContinuation) -> Result<(), String> {
    let Some(path) = pending_restart_file_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {} failed: {error}", parent.display()))?;
    }

    let mut entries: Vec<PendingRestartContinuation> = if path.exists() {
        let raw = fs::read_to_string(&path)
            .map_err(|error| format!("read {} failed: {error}", path.display()))?;
        if raw.trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&raw).unwrap_or_default()
        }
    } else {
        Vec::new()
    };

    entries.push(entry);
    let encoded = serde_json::to_vec_pretty(&entries)
        .map_err(|error| format!("serialize {} failed: {error}", path.display()))?;
    fs::write(&path, encoded).map_err(|error| format!("write {} failed: {error}", path.display()))
}

#[cfg(not(test))]
fn resolve_workspace_root() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("GARYX_WORKSPACE_ROOT").map(PathBuf::from) {
        if root.join("Cargo.toml").exists() {
            return Some(root);
        }
    }

    if let Ok(current) = std::env::current_dir() {
        if current.join("Cargo.toml").exists() && current.join("garyx").exists() {
            return Some(current);
        }
    }

    let exe = std::env::current_exe().ok()?;
    let mut cursor = exe.parent();
    for _ in 0..8 {
        let Some(dir) = cursor else {
            break;
        };
        if dir.join("Cargo.toml").exists() && dir.join("garyx").exists() {
            return Some(dir.to_path_buf());
        }
        cursor = dir.parent();
    }
    None
}

#[cfg(not(test))]
pub async fn build_backend() -> Result<(), String> {
    let Some(root) = resolve_workspace_root() else {
        return Err("failed to locate garyx workspace root (set GARYX_WORKSPACE_ROOT)".to_owned());
    };

    let output = Command::new("cargo")
        .args(["build", "-p", "garyx", "--release"])
        .current_dir(&root)
        .output()
        .await
        .map_err(|error| format!("failed to execute cargo build: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let summary = stderr
            .lines()
            .rev()
            .take(20)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "cargo build failed in {}: {}",
            root.display(),
            summary.trim()
        ));
    }

    install_built_binary(&root)
}

#[cfg(not(test))]
fn install_built_binary(root: &std::path::Path) -> Result<(), String> {
    let src = root.join("target").join("release").join("garyx");
    if !src.exists() {
        return Err(format!("built binary missing: {}", src.display()));
    }

    let home = std::env::var_os("HOME").ok_or_else(|| "HOME is not set".to_owned())?;
    let dest_dir = PathBuf::from(home).join(".cargo").join("bin");
    fs::create_dir_all(&dest_dir)
        .map_err(|error| format!("mkdir {}: {error}", dest_dir.display()))?;

    let final_path = dest_dir.join("garyx");
    let tmp_path = dest_dir.join(".garyx.new");
    // Remove any stale temp from a previous failed install before copying.
    let _ = fs::remove_file(&tmp_path);
    fs::copy(&src, &tmp_path)
        .map_err(|error| format!("copy {} -> {}: {error}", src.display(), tmp_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        fs::set_permissions(&tmp_path, perms)
            .map_err(|error| format!("chmod {}: {error}", tmp_path.display()))?;
    }

    fs::rename(&tmp_path, &final_path).map_err(|error| {
        format!(
            "rename {} -> {}: {error}",
            tmp_path.display(),
            final_path.display()
        )
    })?;
    tracing::info!(path = %final_path.display(), "installed garyx binary");
    Ok(())
}

#[cfg(test)]
pub async fn build_backend() -> Result<(), String> {
    Ok(())
}

/// Request a process restart.
///
/// In unit tests this is a no-op; in runtime builds it schedules a background
/// restart attempt:
/// 1) launchd kickstart (macOS),
/// 2) subprocess respawn fallback.
pub async fn request_restart(reason: String) -> Result<(), String> {
    request_restart_with_options(RestartOptions::new(reason)).await
}

/// Request restart with richer behavior for MCP workflows.
pub async fn request_restart_with_options(options: RestartOptions) -> Result<(), String> {
    #[cfg(test)]
    {
        let _ = options;
        Ok(())
    }

    #[cfg(not(test))]
    {
        if let Some(thread_id) = options
            .continue_thread_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            append_pending_continuation(PendingRestartContinuation {
                thread_id: thread_id.to_owned(),
                message: default_restart_continuation_message().to_owned(),
                reason: options.reason.clone(),
                run_id: options.continue_run_id.clone(),
                created_at_epoch_ms: now_epoch_ms(),
            })?;
        }

        if options.build_before_restart {
            build_backend().await?;
        }

        let exe = std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))?;
        let args: Vec<String> = std::env::args().skip(1).collect();
        let reason = options.reason.clone();

        tokio::spawn(async move {
            tracing::warn!(reason = %reason, "restart requested");

            // Let HTTP handlers return before potentially replacing the process.
            tokio::time::sleep(Duration::from_millis(150)).await;

            if try_launchd_restart().await {
                tracing::info!("restart delegated to launchd; exiting current process");
                // Even though launchd should kill us via kickstart -k, the
                // current process might not be the one launchd is tracking
                // (e.g. if launchd's tracked PID already died).  Exit
                // explicitly so the port is freed for the new instance.
                std::process::exit(0);
            }

            if try_subprocess_restart(&exe, &args).await {
                tracing::warn!("restart subprocess spawned; exiting current process");
                std::process::exit(0);
            }

            tracing::error!("all restart strategies failed");
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(not(test))]
async fn try_launchd_restart() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }

    let uid_out = match Command::new("id").arg("-u").output().await {
        Ok(out) if out.status.success() => out,
        Ok(out) => {
            tracing::warn!(
                status = %out.status,
                stderr = %String::from_utf8_lossy(&out.stderr),
                "failed to resolve uid for launchd target"
            );
            return false;
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to execute `id -u`");
            return false;
        }
    };

    let uid = String::from_utf8_lossy(&uid_out.stdout).trim().to_owned();
    if uid.is_empty() {
        tracing::warn!("empty uid from `id -u`");
        return false;
    }

    let service_target = format!("gui/{uid}/{LAUNCHD_SERVICE_NAME}");

    match Command::new("launchctl")
        .args(["kickstart", "-k", &service_target])
        .output()
        .await
    {
        Ok(out) if out.status.success() => {
            tracing::info!(target = %service_target, "launchctl kickstart -k succeeded");
            true
        }
        Ok(out) => {
            tracing::warn!(
                target = %service_target,
                status = %out.status,
                stderr = %String::from_utf8_lossy(&out.stderr),
                "launchctl kickstart failed"
            );

            // Best-effort fallback for older environments.
            let _ = Command::new("launchctl")
                .args(["stop", LAUNCHD_SERVICE_NAME])
                .output()
                .await;
            match Command::new("launchctl")
                .args(["start", LAUNCHD_SERVICE_NAME])
                .output()
                .await
            {
                Ok(start_out) if start_out.status.success() => true,
                Ok(start_out) => {
                    tracing::warn!(
                        status = %start_out.status,
                        stderr = %String::from_utf8_lossy(&start_out.stderr),
                        "launchctl stop/start fallback failed"
                    );
                    false
                }
                Err(e) => {
                    tracing::warn!(error = %e, "launchctl stop/start fallback errored");
                    false
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to execute launchctl kickstart");
            false
        }
    }
}

#[cfg(not(test))]
async fn try_subprocess_restart(exe: &std::path::Path, args: &[String]) -> bool {
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    match cmd.spawn() {
        Ok(_child) => true,
        Err(e) => {
            tracing::error!(error = %e, "failed to spawn restart subprocess");
            false
        }
    }
}
