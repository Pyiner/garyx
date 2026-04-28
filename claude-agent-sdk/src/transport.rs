use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{ClaudeSDKError, Result};
use crate::types::ClaudeAgentOptions;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Default subprocess transport
// ---------------------------------------------------------------------------

const DEFAULT_MAX_BUFFER_SIZE: usize = 1024 * 1024; // 1 MB

/// Spawns the `claude` CLI as a child process and communicates via JSONL on
/// stdin/stdout.
///
/// stdin and stdout use **separate** mutexes to avoid deadlocks in streaming
/// mode, where the reader blocks on stdout while the writer needs stdin.
pub struct SubprocessTransport {
    options: ClaudeAgentOptions,
    cli_path: String,
    /// Writer half (stdin). Separate lock from reader to prevent deadlock.
    writer: Mutex<Option<tokio::process::ChildStdin>>,
    /// Reader half (stdout). Separate lock from writer to prevent deadlock.
    reader: Mutex<Option<BufReader<tokio::process::ChildStdout>>>,
    /// The child process handle.
    process: Mutex<Option<Child>>,
    ready: AtomicBool,
    max_buffer_size: usize,
    /// True when we are in streaming (bidirectional) mode.
    pub streaming: bool,
}

impl SubprocessTransport {
    /// Create a new `SubprocessTransport`.
    ///
    /// `streaming` controls whether we use `--print --input-format stream-json`
    /// (bidirectional) or `--print` (one-shot) mode.
    pub fn new(options: ClaudeAgentOptions, streaming: bool) -> Self {
        let cli_path = options
            .cli_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| resolve_cli_path(&options));

        let max_buffer_size = options.max_buffer_size.unwrap_or(DEFAULT_MAX_BUFFER_SIZE);

        Self {
            options,
            cli_path,
            writer: Mutex::new(None),
            reader: Mutex::new(None),
            process: Mutex::new(None),
            ready: AtomicBool::new(false),
            max_buffer_size,
            streaming,
        }
    }

    fn build_command(&self, prompt: Option<&str>) -> Command {
        let mut cmd = Command::new(&self.cli_path);
        cmd.kill_on_drop(true);
        for arg in self.options.to_cli_args() {
            cmd.arg(arg);
        }

        if self.streaming {
            cmd.arg("--print");
            cmd.arg("--input-format").arg("stream-json");
        } else if let Some(p) = prompt {
            cmd.arg("--print").arg("--").arg(p);
        }

        // In one-shot (--print) mode, stdin isn't needed (prompt is in args).
        // Using Stdio::null() prevents a deadlock where the CLI waits for
        // stdin to close before exiting.
        if self.streaming {
            cmd.stdin(Stdio::piped());
        } else {
            cmd.stdin(Stdio::null());
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        if let Some(cwd) = &self.options.cwd {
            cmd.current_dir(cwd);
        }

        // Merge env vars
        for (k, v) in &self.options.env {
            cmd.env(k, v);
        }
        cmd.env("CLAUDE_CODE_ENTRYPOINT", "sdk-rs");
        // Remove CLAUDECODE to prevent "cannot be launched inside another Claude
        // Code session" error when running as a subprocess of Claude Code.
        cmd.env_remove("CLAUDECODE");

        if self.options.enable_file_checkpointing {
            cmd.env("CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING", "true");
        }

        cmd
    }

    /// Spawn the CLI process. If `prompt` is `Some`, it is passed as a CLI
    /// argument for one-shot mode.
    pub async fn spawn(&self, prompt: Option<&str>) -> Result<()> {
        let mut proc_guard = self.process.lock().await;
        if proc_guard.is_some() {
            return Ok(());
        }

        let mut cmd = self.build_command(prompt);
        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ClaudeSDKError::NotFound(format!("Claude CLI not found at: {}", self.cli_path))
            } else {
                ClaudeSDKError::Connection(format!("Failed to start Claude CLI: {e}"))
            }
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ClaudeSDKError::Connection("Failed to capture stdout".into()))?;

        // Capture stderr in a background task to log errors from the CLI process.
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut buf = Vec::new();
                let mut reader = tokio::io::BufReader::new(stderr);
                let _ = reader.read_to_end(&mut buf).await;
                if !buf.is_empty() {
                    let text = String::from_utf8_lossy(&buf);
                    tracing::warn!(stderr = %text.trim(), "claude CLI stderr output");
                }
            });
        }

        {
            let mut reader_guard = self.reader.lock().await;
            *reader_guard = Some(BufReader::new(stdout));
        }
        {
            let mut writer_guard = self.writer.lock().await;
            // stdin is None in one-shot mode (Stdio::null()), Some in streaming
            *writer_guard = child.stdin.take();
        }

        *proc_guard = Some(child);
        self.ready.store(true, Ordering::SeqCst);

        Ok(())
    }
}

impl SubprocessTransport {
    pub async fn write(&self, data: &str) -> Result<()> {
        let mut writer_guard = self.writer.lock().await;
        let stdin = writer_guard.as_mut().ok_or(ClaudeSDKError::NotReady)?;

        stdin
            .write_all(data.as_bytes())
            .await
            .map_err(|e| ClaudeSDKError::Connection(format!("Failed to write to stdin: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| ClaudeSDKError::Connection(format!("Failed to flush stdin: {e}")))?;
        Ok(())
    }

    pub async fn read_message(&self) -> Result<Option<Value>> {
        let mut reader_guard = self.reader.lock().await;
        let reader = reader_guard.as_mut().ok_or(ClaudeSDKError::NotReady)?;

        let mut json_buffer = String::new();

        loop {
            let mut line = String::new();
            let n = reader
                .read_line(&mut line)
                .await
                .map_err(|e| ClaudeSDKError::Connection(format!("Read error: {e}")))?;

            if n == 0 {
                // EOF
                if !json_buffer.is_empty() {
                    return match serde_json::from_str::<Value>(&json_buffer) {
                        Ok(v) => Ok(Some(v)),
                        Err(_) => Ok(None),
                    };
                }
                return Ok(None);
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            json_buffer.push_str(trimmed);

            if json_buffer.len() > self.max_buffer_size {
                let len = json_buffer.len();
                json_buffer.clear();
                return Err(ClaudeSDKError::JsonDecode {
                    line: format!("Buffer size {len} exceeds limit {}", self.max_buffer_size),
                    source: serde_json::from_str::<Value>("").unwrap_err(),
                });
            }

            // Speculatively try to parse
            match serde_json::from_str::<Value>(&json_buffer) {
                Ok(v) => return Ok(Some(v)),
                Err(_) => continue,
            }
        }
    }

    pub async fn close(&self) -> Result<()> {
        self.ready.store(false, Ordering::SeqCst);

        // Close stdin
        {
            let mut writer_guard = self.writer.lock().await;
            *writer_guard = None;
        }

        // Close reader
        {
            let mut reader_guard = self.reader.lock().await;
            *reader_guard = None;
        }

        // Kill process
        {
            let mut proc_guard = self.process.lock().await;
            if let Some(mut proc) = proc_guard.take() {
                let _ = proc.kill().await;
                let _ = proc.wait().await;
            }
        }

        Ok(())
    }

    #[cfg(test)]
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    pub async fn end_input(&self) -> Result<()> {
        let mut writer_guard = self.writer.lock().await;
        if let Some(mut stdin) = writer_guard.take() {
            let _ = stdin.shutdown().await;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// CLI discovery
// ---------------------------------------------------------------------------

fn resolve_cli_path(options: &ClaudeAgentOptions) -> String {
    let candidates = cli_candidates(options);
    let required_flags = required_cli_flags(options);

    if required_flags.is_empty() {
        return candidates
            .into_iter()
            .next()
            .unwrap_or_else(|| "claude".to_string());
    }

    for candidate in &candidates {
        if cli_supports_flags(candidate, &required_flags) {
            tracing::info!(
                cli_path = %candidate,
                required_flags = ?required_flags,
                "selected compatible claude CLI candidate"
            );
            return candidate.clone();
        }
    }

    if let Some(candidate) = candidates.into_iter().next() {
        tracing::warn!(
            cli_path = %candidate,
            required_flags = ?required_flags,
            "no compatible claude CLI candidate found; falling back to first discovered candidate"
        );
        return candidate;
    }

    "claude".to_string()
}

fn cli_candidates(options: &ClaudeAgentOptions) -> Vec<String> {
    let mut candidates = Vec::new();

    let has_path_override = if let Some(path_override) = options.env.get("PATH") {
        add_path_candidates(&mut candidates, path_override);
        true
    } else {
        false
    };

    if !has_path_override {
        if let Ok(path) = which::which("claude") {
            push_candidate(&mut candidates, path.to_string_lossy().to_string());
        }
    }

    let home = dirs_home(options);
    let known_paths = [
        format!("{home}/.npm-global/bin/claude"),
        "/usr/local/bin/claude".to_string(),
        format!("{home}/.local/bin/claude"),
        format!("{home}/node_modules/.bin/claude"),
        format!("{home}/.yarn/bin/claude"),
        format!("{home}/.claude/local/claude"),
    ];

    for path in &known_paths {
        push_candidate(&mut candidates, path.clone());
    }

    candidates
}

fn add_path_candidates(candidates: &mut Vec<String>, path_env: &str) {
    for dir in std::env::split_paths(path_env) {
        let candidate = dir.join("claude");
        push_candidate(candidates, candidate.to_string_lossy().to_string());
    }
}

fn push_candidate(candidates: &mut Vec<String>, candidate: String) {
    let path = std::path::Path::new(&candidate);
    if !path.exists() {
        return;
    }

    let normalized = std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string();

    if !candidates.iter().any(|existing| existing == &normalized) {
        candidates.push(normalized);
    }
}

fn required_cli_flags(options: &ClaudeAgentOptions) -> Vec<&'static str> {
    let mut flags = Vec::new();

    if !options.agents.is_empty() {
        flags.push("--agents");
    }

    if options.agent.is_some() {
        flags.push("--agent");
    }

    flags
}

fn cli_supports_flags(cli_path: &str, flags: &[&str]) -> bool {
    if flags.is_empty() {
        return true;
    }

    let output = std::process::Command::new(cli_path).arg("--help").output();
    let Ok(output) = output else {
        return false;
    };

    if !output.status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    flags.iter().all(|flag| stdout.contains(flag))
}

fn dirs_home(options: &ClaudeAgentOptions) -> String {
    options
        .env
        .get("HOME")
        .cloned()
        .or_else(|| options.env.get("USERPROFILE").cloned())
        .or_else(|| std::env::var("HOME").ok())
        .or_else(|| std::env::var("USERPROFILE").ok())
        .unwrap_or_else(|| ".".to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
