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

    fn uid(&self) -> Result<String, Box<dyn std::error::Error>> {
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
        Ok(uid)
    }

    /// Whether the calling process lives in the GUI (Aqua) login session.
    ///
    /// Only an Aqua-session process can `bootstrap` into the `gui/<uid>`
    /// domain; over SSH / headless logins the manager is the per-user
    /// background domain and the GUI domain is unreachable (launchctl rejects
    /// it with "Domain does not support specified action"). `managername`
    /// reports the current process's domain manager, which is exactly the
    /// signal that decides which domain we can install into.
    fn is_aqua_session(&self) -> bool {
        ProcessCommand::new(LAUNCHCTL_BIN)
            .arg("managername")
            .output()
            .map(|out| {
                out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "Aqua"
            })
            .unwrap_or(false)
    }

    fn domain_exists(&self, domain: &str) -> bool {
        ProcessCommand::new(LAUNCHCTL_BIN)
            .args(["print", domain])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    /// Domains to attempt when bootstrapping a not-yet-loaded agent, in
    /// priority order. Prefer `gui/<uid>` when an Aqua login domain exists,
    /// even if this command is invoked from an SSH / Background session: macOS
    /// accepts bootstrapping into that existing GUI domain, while bootstrapping
    /// the same LaunchAgent into `user/<uid>` can fail with launchctl error 5.
    /// On truly headless sessions, fall back to the per-user domain.
    fn candidate_install_domains(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let uid = self.uid()?;
        Ok(candidate_install_domains_for(
            &uid,
            self.is_aqua_session(),
            self.domain_exists(&format!("gui/{uid}")),
        ))
    }

    /// The domain the agent is currently loaded in, if any. Probing both
    /// domains lets stop / restart / uninstall act on the live service
    /// regardless of which session type installed it.
    fn loaded_domain(&self) -> Option<String> {
        let uid = self.uid().ok()?;
        for domain in [format!("gui/{uid}"), format!("user/{uid}")] {
            let target = format!("{domain}/{LAUNCHD_SERVICE_NAME}");
            let loaded = ProcessCommand::new(LAUNCHCTL_BIN)
                .args(["print", &target])
                .output()
                .map(|out| out.status.success())
                .unwrap_or(false);
            if loaded {
                return Some(domain);
            }
        }
        None
    }

    fn target(&self) -> Result<String, Box<dyn std::error::Error>> {
        // Prefer where the agent actually lives; otherwise fall back to the
        // domain we would install into, so commands run before the first
        // bootstrap still resolve a sensible target.
        if let Some(domain) = self.loaded_domain() {
            return Ok(format!("{domain}/{LAUNCHD_SERVICE_NAME}"));
        }
        let domain = self
            .candidate_install_domains()?
            .into_iter()
            .next()
            .ok_or("no launchd domain available for the current session")?;
        Ok(format!("{domain}/{LAUNCHD_SERVICE_NAME}"))
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
        if self.loaded_domain().is_some() {
            return Ok(());
        }

        let plist_arg = plist_path.display().to_string();
        let mut last_err: Option<String> = None;
        for domain in self.candidate_install_domains()? {
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
            // Fall through to the next candidate domain — e.g. an Aqua session
            // whose GUI domain rejected the bootstrap can still land in the
            // per-user domain.
            last_err = Some(format!(
                "launchctl bootstrap {domain} failed: {}",
                stderr.trim()
            ));
        }
        Err(last_err
            .unwrap_or_else(|| "launchctl bootstrap failed: no domain available".to_owned())
            .into())
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

fn candidate_install_domains_for(
    uid: &str,
    is_aqua_session: bool,
    gui_domain_exists: bool,
) -> Vec<String> {
    let gui_domain = format!("gui/{uid}");
    let user_domain = format!("user/{uid}");
    if is_aqua_session || gui_domain_exists {
        vec![gui_domain, user_domain]
    } else {
        vec![user_domain]
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
