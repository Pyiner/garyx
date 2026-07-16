//! Runtime restart helpers.
//!
//! Provides best-effort restart orchestration for `/api/restart` and service
//! management flows.

#[cfg(not(test))]
use std::process::Stdio;
#[cfg(not(test))]
use std::time::Duration;

#[cfg(not(test))]
use tokio::process::Command;

#[cfg(not(test))]
const LAUNCHD_SERVICE_NAME: &str = "com.garyx.agent";

/// Request a process restart.
///
/// In unit tests this is a no-op; in runtime builds it schedules a background
/// restart attempt:
/// 1) launchd kickstart (macOS),
/// 2) subprocess respawn fallback.
pub async fn request_restart(reason: String) -> Result<(), String> {
    #[cfg(test)]
    {
        let _ = reason;
        Ok(())
    }

    #[cfg(not(test))]
    {
        let exe = std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))?;
        let args: Vec<String> = std::env::args().skip(1).collect();

        tokio::spawn(async move {
            tracing::warn!(reason = %reason, "restart requested");

            // Let HTTP handlers return before potentially replacing the process.
            tokio::time::sleep(Duration::from_millis(150)).await;

            let force_subprocess = cfg!(debug_assertions)
                && std::env::var_os("GARYX_TEST_FORCE_SUBPROCESS_RESTART").is_some();
            if !force_subprocess && try_launchd_restart().await {
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

    // The agent may be bootstrapped into either the GUI (`gui/<uid>`, desktop
    // login) or the per-user (`user/<uid>`, SSH / headless) domain. Kickstart
    // must target the domain it actually lives in, so probe both rather than
    // assuming GUI. Falls back to the GUI target if neither prints (e.g. a
    // dev `gateway run` not managed by launchd), where the stop/start and
    // subprocess paths below still apply.
    let service_target = resolve_loaded_target(&uid)
        .await
        .unwrap_or_else(|| format!("gui/{uid}/{LAUNCHD_SERVICE_NAME}"));

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

/// Return the `<domain>/<uid>/<service>` target the agent is currently loaded
/// in, probing the GUI domain first and then the per-user domain. `None` when
/// the service is not loaded in either (the caller then assumes a sensible
/// default and relies on its further fallbacks).
#[cfg(not(test))]
async fn resolve_loaded_target(uid: &str) -> Option<String> {
    for domain in [format!("gui/{uid}"), format!("user/{uid}")] {
        let target = format!("{domain}/{LAUNCHD_SERVICE_NAME}");
        let loaded = Command::new("launchctl")
            .args(["print", &target])
            .output()
            .await
            .map(|out| out.status.success())
            .unwrap_or(false);
        if loaded {
            return Some(target);
        }
    }
    None
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
