//! Linux systemd (user) backend for the gateway service.
//!
//! Writes `~/.config/systemd/user/garyx.service` and drives it via
//! `systemctl --user`. No root/sudo required — runs under the current user.
//!
//! For the service to keep running after you log out (the usual case on a
//! shared dev box) the user needs `loginctl enable-linger`. We detect this
//! at install time and surface a warning rather than silently letting the
//! service die on logout.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use super::{InstallReport, ServiceManager, ServiceSpec};

const SYSTEMD_UNIT_NAME: &str = "garyx.service";
const SYSTEMCTL_BIN: &str = "systemctl";
const LOGINCTL_BIN: &str = "loginctl";

pub(crate) struct SystemdUserManager;

impl SystemdUserManager {
    pub(crate) fn new() -> Self {
        Self
    }

    fn unit_path(&self) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let home = std::env::var("HOME").map_err(|_| "HOME is not set")?;
        Ok(PathBuf::from(home)
            .join(".config")
            .join("systemd")
            .join("user")
            .join(SYSTEMD_UNIT_NAME))
    }

    fn env_file_path(&self) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let home = std::env::var("HOME").map_err(|_| "HOME is not set")?;
        Ok(PathBuf::from(home).join(".garyx").join("env"))
    }

    fn write_unit(&self, spec: &ServiceSpec) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let unit_path = self.unit_path()?;
        fs::create_dir_all(&spec.log_dir)?;
        if let Some(parent) = unit_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let env_file = self.env_file_path()?;
        let contents = render_unit_file(
            &spec.host,
            spec.port,
            &spec.log_dir,
            spec.workspace_root.as_deref(),
            &env_file,
        );
        let needs_write = fs::read_to_string(&unit_path)
            .map(|existing| existing != contents)
            .unwrap_or(true);
        if needs_write {
            fs::write(&unit_path, contents)?;
        }
        Ok(unit_path)
    }

    fn systemctl(&self, args: &[&str]) -> Result<std::process::Output, Box<dyn std::error::Error>> {
        let output = ProcessCommand::new(SYSTEMCTL_BIN)
            .arg("--user")
            .args(args)
            .output()
            .map_err(|err| -> Box<dyn std::error::Error> {
                if err.kind() == std::io::ErrorKind::NotFound {
                    format!(
                        "`{SYSTEMCTL_BIN}` not found — this backend requires systemd. Install systemd or run `garyx gateway run` manually."
                    )
                    .into()
                } else {
                    err.into()
                }
            })?;
        Ok(output)
    }

    fn run_systemctl(&self, args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
        let output = self.systemctl(args)?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut msg = format!("`systemctl --user {}` failed", args.join(" "));
        if !stderr.trim().is_empty() {
            msg.push_str(": ");
            msg.push_str(stderr.trim());
        } else if !stdout.trim().is_empty() {
            msg.push_str(": ");
            msg.push_str(stdout.trim());
        }
        if stderr.contains("Failed to connect to bus") {
            msg.push_str("\n  hint: make sure XDG_RUNTIME_DIR=/run/user/$(id -u) is set (try `export XDG_RUNTIME_DIR=/run/user/$(id -u)` or log in via `ssh -t` for a proper session)");
        }
        Err(msg.into())
    }

    fn linger_enabled(&self) -> bool {
        let uid_output = ProcessCommand::new("id").arg("-un").output();
        let Ok(uid) = uid_output else {
            return false;
        };
        if !uid.status.success() {
            return false;
        }
        let username = String::from_utf8_lossy(&uid.stdout).trim().to_owned();
        if username.is_empty() {
            return false;
        }
        let output = ProcessCommand::new(LOGINCTL_BIN)
            .args(["show-user", &username, "--value", "--property=Linger"])
            .output();
        let Ok(output) = output else {
            return false;
        };
        if !output.status.success() {
            return false;
        }
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .eq_ignore_ascii_case("yes")
    }

    fn linger_warning(&self) -> Option<String> {
        if self.linger_enabled() {
            return None;
        }
        let username = ProcessCommand::new("id")
            .arg("-un")
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    Some(String::from_utf8_lossy(&o.stdout).trim().to_owned())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "$USER".to_owned());
        Some(format!(
            "user linger is OFF — the gateway will stop when you log out. Enable it with:\n    sudo loginctl enable-linger {username}"
        ))
    }
}

impl ServiceManager for SystemdUserManager {
    fn backend_name(&self) -> &'static str {
        "systemd (--user)"
    }

    fn install(&self, spec: &ServiceSpec) -> Result<InstallReport, Box<dyn std::error::Error>> {
        let unit_path = self.write_unit(spec)?;
        self.run_systemctl(&["daemon-reload"])?;
        // enable --now covers both first install and re-install: it (re)writes
        // the wants symlink and starts the unit if not running.
        self.run_systemctl(&["enable", "--now", SYSTEMD_UNIT_NAME])?;
        // If it was already running, enable --now doesn't restart it after the
        // unit file changed. Restart to pick up any new ExecStart / env.
        self.run_systemctl(&["restart", SYSTEMD_UNIT_NAME])?;

        let mut warnings = Vec::new();
        if let Some(w) = self.linger_warning() {
            warnings.push(w);
        }
        Ok(InstallReport {
            unit_path,
            backend: self.backend_name(),
            warnings,
        })
    }

    fn uninstall(&self) -> Result<(), Box<dyn std::error::Error>> {
        // `disable --now` stops + removes the wants symlink. Ignore failure if
        // the unit is already absent; we still want to clean up.
        let _ = self.run_systemctl(&["disable", "--now", SYSTEMD_UNIT_NAME]);
        let unit_path = self.unit_path()?;
        if unit_path.exists() {
            fs::remove_file(&unit_path)?;
        }
        let _ = self.run_systemctl(&["daemon-reload"]);
        Ok(())
    }

    fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        let unit_path = self.unit_path()?;
        if !unit_path.exists() {
            return Err(format!(
                "systemd unit not installed: {} is missing — run `garyx gateway install` first",
                unit_path.display()
            )
            .into());
        }
        self.run_systemctl(&["start", SYSTEMD_UNIT_NAME])?;
        Ok(())
    }

    fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
        // stop on a nonexistent unit fails loudly; only complain if we have a
        // unit file but the stop still errored.
        let unit_path = self.unit_path()?;
        if !unit_path.exists() {
            return Ok(());
        }
        self.run_systemctl(&["stop", SYSTEMD_UNIT_NAME])?;
        Ok(())
    }

    fn restart(&self, spec: &ServiceSpec) -> Result<InstallReport, Box<dyn std::error::Error>> {
        let unit_path = self.write_unit(spec)?;
        self.run_systemctl(&["daemon-reload"])?;
        self.run_systemctl(&["enable", SYSTEMD_UNIT_NAME])?;
        self.run_systemctl(&["restart", SYSTEMD_UNIT_NAME])?;

        let mut warnings = Vec::new();
        if let Some(w) = self.linger_warning() {
            warnings.push(w);
        }
        Ok(InstallReport {
            unit_path,
            backend: self.backend_name(),
            warnings,
        })
    }

    fn is_installed(&self) -> bool {
        self.unit_path().map(|p| p.exists()).unwrap_or(false)
    }
}

/// Render a systemd user-unit file for the gateway.
///
/// Notes:
/// - `ExecStart` goes through the user's login shell (`getent passwd %u`)
///   in login+interactive mode (`-lic`) so the gateway inherits the same
///   PATH / env you get when you `ssh` into the box. Without this, the
///   service starts from the minimal systemd env and can't find provider
///   CLIs like `claude` installed under `~/.npm-global/bin`.
///   `%u` is the systemd-expanded username; `exec` chains keep the parent
///   chain clean (no leftover sh/zsh processes). `garyx` is resolved via
///   the login shell's PATH rather than baked as an absolute path, so a
///   later reinstall to a different location is picked up on restart
///   without regenerating the unit.
/// - `EnvironmentFile=-%h/.garyx/env` is optional: users can drop API keys
///   (e.g. `CLAUDE_CODE_OAUTH_TOKEN=...`) in that file and systemd will pick
///   them up. The leading `-` means "skip if missing" — no error otherwise.
/// - Logs go to `log_dir/stdout.log` and `log_dir/stderr.log` via
///   `append:` so restarts don't truncate history.
fn render_unit_file(
    host: &str,
    port: u16,
    log_dir: &Path,
    workspace_root: Option<&Path>,
    env_file: &Path,
) -> String {
    let workspace_line = workspace_root
        .map(|root| format!("Environment=GARYX_WORKSPACE_ROOT={}\n", root.display()))
        .unwrap_or_default();
    format!(
        "[Unit]
Description=Garyx AI Gateway
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/bin/sh -c 'exec \"$(getent passwd %u | cut -d: -f7)\" -lic \"exec garyx gateway run --host {host} --port {port}\"'
Restart=on-failure
RestartSec=5
{workspace_line}EnvironmentFile=-{env_file}
StandardOutput=append:{log_dir}/stdout.log
StandardError=append:{log_dir}/stderr.log

[Install]
WantedBy=default.target
",
        host = host,
        port = port,
        workspace_line = workspace_line,
        env_file = env_file.display(),
        log_dir = log_dir.display(),
    )
}

#[cfg(test)]
mod tests;
