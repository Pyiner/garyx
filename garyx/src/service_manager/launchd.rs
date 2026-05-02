//! macOS launchd (LaunchAgent) backend for the gateway service.
//!
//! Writes `~/Library/LaunchAgents/com.garyx.agent.plist` and drives the
//! service via `/bin/launchctl`. The plist resolves the user's login shell
//! through Directory Services (`dscl`) and re-enters it in login+interactive
//! mode so the service inherits the same PATH / env the user gets in their
//! Terminal session — provider CLIs like `claude` installed under
//! `~/.npm-global/bin` become discoverable while the gateway binary itself is
//! pinned to the absolute path used when the service spec is refreshed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use super::{InstallReport, ServiceManager, ServiceSpec};

const LAUNCHD_SERVICE_NAME: &str = "com.garyx.agent";
const LAUNCHCTL_BIN: &str = "/bin/launchctl";

pub(crate) struct LaunchdManager;

impl LaunchdManager {
    pub(crate) fn new() -> Self {
        Self
    }

    fn domain(&self) -> Result<String, Box<dyn std::error::Error>> {
        let output = ProcessCommand::new("id").arg("-u").output()?;
        if !output.status.success() {
            return Err(format!(
                "failed to resolve uid: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )
            .into());
        }
        let uid = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if uid.is_empty() {
            return Err("empty uid from `id -u`".into());
        }
        Ok(format!("gui/{uid}"))
    }

    fn target(&self) -> Result<String, Box<dyn std::error::Error>> {
        Ok(format!("{}/{}", self.domain()?, LAUNCHD_SERVICE_NAME))
    }

    fn plist_path(&self) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let home = std::env::var("HOME").map_err(|_| "HOME is not set")?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LAUNCHD_SERVICE_NAME}.plist")))
    }

    fn write_plist(&self, spec: &ServiceSpec) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let plist_path = self.plist_path()?;
        fs::create_dir_all(&spec.log_dir)?;
        if let Some(parent) = plist_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let stdout_path = spec.log_dir.join("stdout.log");
        let stderr_path = spec.log_dir.join("stderr.log");
        let contents = render_launch_agent_plist(
            &spec.binary_path,
            &spec.host,
            spec.port,
            &stdout_path,
            &stderr_path,
            spec.workspace_root.as_deref(),
        );
        let needs_write = fs::read_to_string(&plist_path)
            .map(|existing| existing != contents)
            .unwrap_or(true);
        if needs_write {
            fs::write(&plist_path, contents)?;
        }
        Ok(plist_path)
    }

    fn ensure_bootstrapped(&self, plist_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let target = self.target()?;
        let print_output = ProcessCommand::new(LAUNCHCTL_BIN)
            .args(["print", &target])
            .output()?;
        if print_output.status.success() {
            return Ok(());
        }

        let domain = self.domain()?;
        let plist_arg = plist_path.display().to_string();
        let output = ProcessCommand::new(LAUNCHCTL_BIN)
            .args(["bootstrap", &domain, &plist_arg])
            .output()?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("service already loaded") {
            return Ok(());
        }
        Err(format!("launchctl bootstrap failed: {}", stderr.trim()).into())
    }

    fn bootout(&self) -> Result<(), Box<dyn std::error::Error>> {
        let target = self.target()?;
        let output = ProcessCommand::new(LAUNCHCTL_BIN)
            .args(["bootout", &target])
            .output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.contains("No such process") && !stderr.contains("service not found") {
                return Err(format!("launchctl bootout failed: {}", stderr.trim()).into());
            }
        }
        Ok(())
    }

    fn kickstart(&self, kill_existing: bool) -> Result<(), Box<dyn std::error::Error>> {
        let target = self.target()?;
        let mut args = vec!["kickstart"];
        if kill_existing {
            args.push("-k");
        }
        args.push(&target);
        let output = ProcessCommand::new(LAUNCHCTL_BIN).args(&args).output()?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("launchctl kickstart failed: {}", stderr.trim()).into())
    }
}

impl ServiceManager for LaunchdManager {
    fn backend_name(&self) -> &'static str {
        "launchd"
    }

    fn install(&self, spec: &ServiceSpec) -> Result<InstallReport, Box<dyn std::error::Error>> {
        let plist_path = self.write_plist(spec)?;
        self.ensure_bootstrapped(&plist_path)?;
        self.kickstart(false)?;
        Ok(InstallReport {
            unit_path: plist_path,
            backend: self.backend_name(),
            warnings: Vec::new(),
        })
    }

    fn uninstall(&self) -> Result<(), Box<dyn std::error::Error>> {
        // bootout first so launchd stops tracking the job, then rm the plist.
        self.bootout()?;
        let plist_path = self.plist_path()?;
        if plist_path.exists() {
            fs::remove_file(&plist_path)?;
        }
        Ok(())
    }

    fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        let plist_path = self.plist_path()?;
        if !plist_path.exists() {
            return Err(format!(
                "launch agent not installed: {} is missing — run `garyx gateway install` first",
                plist_path.display()
            )
            .into());
        }
        self.ensure_bootstrapped(&plist_path)?;
        self.kickstart(false)?;
        Ok(())
    }

    fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.bootout()
    }

    fn restart(&self, spec: &ServiceSpec) -> Result<InstallReport, Box<dyn std::error::Error>> {
        let plist_path = self.write_plist(spec)?;
        self.ensure_bootstrapped(&plist_path)?;
        self.kickstart(true)?;
        Ok(InstallReport {
            unit_path: plist_path,
            backend: self.backend_name(),
            warnings: Vec::new(),
        })
    }

    fn is_installed(&self) -> bool {
        self.plist_path().map(|p| p.exists()).unwrap_or(false)
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn render_launch_agent_plist(
    binary_path: &Path,
    host: &str,
    port: u16,
    stdout_path: &Path,
    stderr_path: &Path,
    workspace_root: Option<&Path>,
) -> String {
    // Launchd starts user agents with a minimal env. To match what the user
    // sees in Terminal / SSH, we look up the user's login shell via
    // Directory Services and re-enter it in login+interactive mode (`-lic`)
    // so `.zshenv` / `.zprofile` / `.zshrc` are sourced and PATH picks up
    // `~/.npm-global/bin`, cargo, pyenv, etc. The gateway binary path is
    // pinned so a freshly installed binary is what launchd restarts.
    // `exec` chains keep the parent chain clean (garyx <- launchd, no sh/zsh
    // shims).
    let binary_arg = super::shell_double_quoted_arg_for_nested_command(binary_path);
    let command = format!(
        r#"exec "$(dscl . -read /Users/$(id -un) UserShell | awk '/^UserShell:/ {{print $NF}}')" -lic "exec {binary_arg} gateway run --host {host} --port {port}""#,
        binary_arg = binary_arg,
        host = host,
        port = port,
    );
    let workspace_entry = workspace_root.map(|root| {
        format!(
            "    <key>GARYX_WORKSPACE_ROOT</key>\n    <string>{}</string>\n",
            xml_escape(&root.display().to_string())
        )
    });
    let env_entries = [workspace_entry].into_iter().flatten().collect::<String>();
    let env_block = if env_entries.is_empty() {
        String::new()
    } else {
        format!("  <key>EnvironmentVariables</key>\n  <dict>\n{env_entries}  </dict>\n")
    };
    let command = xml_escape(&command);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/sh</string>
    <string>-c</string>
    <string>{command}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ThrottleInterval</key>
  <integer>35</integer>
  <key>LimitLoadToSessionType</key>
  <string>Aqua</string>
{env_block}  <key>SoftResourceLimits</key>
  <dict>
    <key>NumberOfFiles</key>
    <integer>65536</integer>
  </dict>
  <key>HardResourceLimits</key>
  <dict>
    <key>NumberOfFiles</key>
    <integer>65536</integer>
  </dict>
  <key>StandardOutPath</key>
  <string>{stdout_path}</string>
  <key>StandardErrorPath</key>
  <string>{stderr_path}</string>
</dict>
</plist>
"#,
        label = LAUNCHD_SERVICE_NAME,
        command = command,
        stdout_path = xml_escape(&stdout_path.display().to_string()),
        stderr_path = xml_escape(&stderr_path.display().to_string()),
    )
}

#[cfg(test)]
mod tests;
