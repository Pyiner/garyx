//! Spawn a plugin child process and wire its stdio to [`Transport`].
//!
//! The host talks JSON-RPC over the child's stdin/stdout. Stderr is
//! captured and forwarded to `tracing`. The plugin's exit is exposed as
//! an awaitable future so the manager can trigger respawn without
//! racing the transport drain.
//!
//! Shutdown sequence (`shutdown` → SIGTERM → SIGKILL) follows §6.3 of
//! the protocol doc. The stop-one-account sequence (`stop` → optional
//! `abandon_inbound` response → ACK) is the manager's job; this module
//! only knows how to start, bring down, and reap a single child.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use super::codec::CodecError;
use super::manifest::PluginManifest;
use super::transport::{
    InboundHandler, PluginRpcClient, RpcError, Transport, TransportConfig, TransportHandles,
};

/// §6.3: grace between `SIGTERM` and `SIGKILL`. The host always waits
/// this long regardless of `manifest.runtime.shutdown_grace_ms` (which
/// governs only the *shutdown-RPC* drain window).
const SIGTERM_GRACE: Duration = Duration::from_secs(2);

/// §11.1: host-enforced timeout for the `shutdown` RPC itself. The
/// manifest's `shutdown_grace_ms` controls the post-RPC exit-wait
/// budget, *not* how long the RPC round trip is allowed to take —
/// those are two separate budgets and the spec nails the RPC one at
/// 10s regardless of manifest tuning so a misconfigured plugin can't
/// prolong shutdown.
const SHUTDOWN_RPC_TIMEOUT: Duration = Duration::from_secs(10);

/// Upper bound on how long we wait for the kernel to deliver our
/// `SIGKILL`'s exit notification. In practice this resolves within
/// microseconds; the timeout exists only so a wedged kernel test
/// fixture cannot hang shutdown forever.
const SIGKILL_REAP_BUDGET: Duration = Duration::from_secs(5);

/// Errors that can occur while spawning, driving, or shutting down a
/// plugin child. RPC errors propagate as-is from [`RpcError`]; this
/// enum covers everything *around* the transport.
#[derive(Debug, Error)]
pub enum SubprocessError {
    #[error("binary not found or not executable: {0}")]
    BinaryMissing(PathBuf),
    #[error("failed to spawn plugin binary: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("child missing stdio handle after spawn (stdin/stdout)")]
    StdioMissing,
    #[error("plugin child exited before it produced any output")]
    StartupExit,
    #[error("rpc: {0}")]
    Rpc(#[from] RpcError),
}

/// Options that affect *how* the child runs (environment, working
/// directory, stderr sink). Manifest-derived knobs such as
/// `max_frame_bytes` go through the [`TransportConfig`].
#[derive(Debug, Clone, Default)]
pub struct SpawnOptions {
    /// Extra env vars applied on top of the manifest's entry.env.
    /// Precedence: caller > manifest > parent process.
    pub extra_env: Vec<(String, String)>,
    /// If set, child's working directory. Otherwise inherits the host's
    /// cwd.
    pub working_dir: Option<PathBuf>,
    /// Field names whose values the stderr drainer must redact before
    /// re-emitting (§8.4). Typically populated from the plugin's
    /// `describe.schema` by walking `x-garyx.secret: true` markers.
    pub stderr_redactions: BTreeSet<String>,
}

/// A running plugin child.
///
/// Clone-cheap RPC access is via [`Self::client`]. The owner of this
/// struct is responsible for graceful shutdown; [`Drop`] only escalates
/// to SIGKILL as a safety net.
pub struct SubprocessPlugin {
    plugin_id: String,
    shutdown_grace: Duration,
    client: PluginRpcClient,
    /// Writer-task handle — kept alive so the task doesn't cancel before
    /// shutdown drains pending frames.
    #[allow(dead_code)]
    writer_task: JoinHandle<()>,
    /// Supervisor around the reader task. Owns the raw reader
    /// `JoinHandle` and escalates a fatal protocol error to `SIGKILL` so
    /// the child cannot sit half-dead after the reader exits. Codex pass
    /// 4 called out that without this, an envelope violation would kill
    /// the reader, leave the child running, and defeat supervision until
    /// the plugin either crashed on its own or wrote to stdin.
    #[allow(dead_code)]
    reader_supervisor: JoinHandle<()>,
    #[allow(dead_code)]
    stderr_task: JoinHandle<()>,
    /// `None` after `wait_for_exit` has consumed it.
    exit_rx: Option<oneshot::Receiver<ExitReport>>,
    /// Mirror of the exit signal. Multiple consumers can observe the
    /// latest exit state without racing on the oneshot.
    exit_watch: watch::Receiver<Option<ExitReport>>,
    /// Child handle kept only for SIGKILL fallback. A dedicated task
    /// (`exit_task`) owns the `.wait()` call.
    child_kill: ChildKill,
    #[allow(dead_code)]
    exit_task: JoinHandle<()>,
}

/// A partial report of how the child exited. Exposed to the manager so
/// it can decide whether to respawn (Crash) or leave things as-is
/// (GracefulExit after shutdown RPC).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitReport {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub success: bool,
}

impl ExitReport {
    fn from_status(status: std::process::ExitStatus) -> Self {
        #[cfg(unix)]
        let signal = std::os::unix::process::ExitStatusExt::signal(&status);
        #[cfg(not(unix))]
        let signal: Option<i32> = None;
        Self {
            code: status.code(),
            signal,
            success: status.success(),
        }
    }
}

/// Handle used to escalate shutdown. The PID sits behind an
/// `Arc<AtomicI32>` so the exit-observing task can disarm it the
/// moment the kernel reports reap — from that point on any further
/// `signal()` call is a no-op.
///
/// Pass 3 of codex review called out the race: if the kernel recycled
/// the PID after reap and `Drop` then sent `SIGKILL`, we could kill an
/// unrelated process. Disarming in the exit path closes the realistic
/// window (which is milliseconds to seconds post-reap). A residual
/// sub-microsecond window between `child.wait()` returning and our
/// atomic store is intentionally ignored — closing it would require
/// holding the `Child` in `Drop` alongside the exit task.
#[derive(Clone)]
struct ChildKill {
    /// Sentinel `-1` means disarmed / never had a PID.
    pid: Arc<AtomicI32>,
}

impl ChildKill {
    fn new(pid: Option<u32>) -> Self {
        let raw = pid.map(|p| p as i32).unwrap_or(-1);
        Self {
            pid: Arc::new(AtomicI32::new(raw)),
        }
    }

    fn disarm(&self) {
        self.pid.store(-1, Ordering::Release);
    }

    #[cfg(unix)]
    fn signal(&self, sig: i32) {
        let pid = self.pid.load(Ordering::Acquire);
        if pid <= 0 {
            return;
        }
        // SAFETY: `kill(2)` is async-signal-safe. `pid` was taken from
        // our child at spawn time and has not been disarmed, so either
        // (a) the child is still alive and receives `sig`, or (b) the
        // kernel has the PID in a zombie state and `kill` is a no-op.
        // A residual race with post-reap PID reuse is documented above.
        unsafe {
            libc::kill(pid as libc::pid_t, sig);
        }
    }

    #[cfg(not(unix))]
    fn signal(&self, _sig: i32) {
        // On Windows we'd use TerminateProcess — not needed for the
        // initial milestone (channels always land on a Unix host first).
    }
}

impl SubprocessPlugin {
    /// Spawn a child process per `manifest` and wire up the transport.
    /// Does NOT call `initialize`/`describe`/`start` — the caller
    /// chooses the sequence so the same primitive serves both the
    /// preflight dry-run and the real lifecycle.
    pub fn spawn<H>(
        manifest: &PluginManifest,
        options: SpawnOptions,
        handler: Arc<H>,
    ) -> Result<Self, SubprocessError>
    where
        H: InboundHandler,
    {
        let binary = manifest.binary_path();
        if !binary.exists() {
            return Err(SubprocessError::BinaryMissing(binary));
        }

        let mut cmd = Command::new(&binary);
        cmd.args(&manifest.entry.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in &manifest.entry.env {
            cmd.env(k, v);
        }
        for (k, v) in &options.extra_env {
            cmd.env(k, v);
        }
        if let Some(dir) = &options.working_dir {
            cmd.current_dir(dir);
        } else {
            cmd.current_dir(&manifest.manifest_dir);
        }

        let mut child: Child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or(SubprocessError::StdioMissing)?;
        let stdout = child.stdout.take().ok_or(SubprocessError::StdioMissing)?;
        let stderr = child.stderr.take().ok_or(SubprocessError::StdioMissing)?;
        let pid = child.id();

        let child_kill = ChildKill::new(pid);

        let transport_cfg = TransportConfig {
            plugin_id: manifest.plugin.id.clone(),
            max_frame_bytes: manifest.runtime.max_frame_bytes,
            ..Default::default()
        };
        let (
            client,
            TransportHandles {
                reader: reader_task,
                writer: writer_task,
            },
        ) = Transport::spawn(stdout, stdin, transport_cfg, handler);

        // Reader supervisor: turn a fatal protocol error into a child
        // termination so the supervision path actually sees it. See the
        // comment on `SubprocessPlugin::reader_supervisor` and §11.1 of
        // the protocol doc.
        let reader_supervisor = tokio::spawn({
            let plugin_id = manifest.plugin.id.clone();
            let child_kill = child_kill.clone();
            async move {
                match reader_task.await {
                    Ok(Ok(())) => {
                        // `run_reader` only returns via `Err`, so this
                        // arm is unreachable today. Handle it defensively
                        // in case the reader is ever given an explicit
                        // success path — silence + let exit_task fire.
                        debug!(plugin = %plugin_id, "reader exited with Ok(()) — letting child close naturally");
                    }
                    Ok(Err(CodecError::UnexpectedEof)) => {
                        // Clean close. The child is either already dead
                        // or about to be; don't force it.
                        debug!(plugin = %plugin_id, "reader hit peer EOF; letting child close naturally");
                    }
                    Ok(Err(err)) => {
                        warn!(
                            plugin = %plugin_id,
                            error = %err,
                            "reader exited on a fatal protocol error; sending SIGKILL so supervision observes exit",
                        );
                        child_kill.signal(libc_kill());
                    }
                    Err(join_err) => {
                        // Reader task panicked. Treat as fatal.
                        error!(
                            plugin = %plugin_id,
                            error = %join_err,
                            "reader task panicked; sending SIGKILL so supervision observes exit",
                        );
                        child_kill.signal(libc_kill());
                    }
                }
            }
        });

        let stderr_task = tokio::spawn(drain_stderr(
            stderr,
            manifest.plugin.id.clone(),
            options.stderr_redactions.clone(),
        ));

        let (exit_tx, exit_rx) = oneshot::channel::<ExitReport>();
        let (exit_watch_tx, exit_watch_rx) = watch::channel::<Option<ExitReport>>(None);
        let plugin_id = manifest.plugin.id.clone();
        let exit_kill_disarm = child_kill.clone();
        let exit_task = tokio::spawn(async move {
            let report = match child.wait().await {
                Ok(status) => ExitReport::from_status(status),
                Err(err) => {
                    warn!(plugin = %plugin_id, error = %err, "plugin transport: child.wait failed");
                    ExitReport {
                        code: None,
                        signal: None,
                        success: false,
                    }
                }
            };
            // Child is reaped — the kernel may recycle this PID at any
            // point from here on. Disarm before we let any other task
            // race with Drop.
            exit_kill_disarm.disarm();
            if report.success {
                info!(plugin = %plugin_id, ?report, "plugin child exited");
            } else {
                warn!(plugin = %plugin_id, ?report, "plugin child exited non-success");
            }
            let _ = exit_watch_tx.send(Some(report.clone()));
            let _ = exit_tx.send(report);
        });

        Ok(Self {
            plugin_id: manifest.plugin.id.clone(),
            shutdown_grace: Duration::from_millis(manifest.runtime.shutdown_grace_ms),
            client,
            writer_task,
            reader_supervisor,
            stderr_task,
            exit_rx: Some(exit_rx),
            exit_watch: exit_watch_rx,
            child_kill,
            exit_task,
        })
    }

    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    /// RPC handle. Clone for dispatchers / auth flow machinery.
    pub fn client(&self) -> PluginRpcClient {
        self.client.clone()
    }

    /// Take the one-shot exit future. Only one caller can observe this;
    /// subsequent callers should use [`Self::exit_watch`].
    pub fn take_exit_future(&mut self) -> Option<oneshot::Receiver<ExitReport>> {
        self.exit_rx.take()
    }

    /// Observable for the exit state. `None` while the child is alive;
    /// `Some(ExitReport)` after exit.
    pub fn exit_watch(&self) -> watch::Receiver<Option<ExitReport>> {
        self.exit_watch.clone()
    }

    /// Execute the §6.3 shutdown sequence:
    ///
    /// 1. Send `shutdown` RPC bounded by [`SHUTDOWN_RPC_TIMEOUT`] (10s,
    ///    §11.1), then wait up to `runtime.shutdown_grace_ms` (default
    ///    3s) for the child to exit. The two budgets are independent:
    ///    the RPC timeout is protocol-fixed so a misconfigured
    ///    manifest cannot prolong shutdown, and the exit wait is the
    ///    spec's drain window.
    /// 2. If it doesn't exit, `SIGTERM` and wait `SIGTERM_GRACE` (2s).
    ///    The 2s is *protocol-fixed*, not manifest-tunable.
    /// 3. If it still hasn't exited, `SIGKILL` and wait for the
    ///    kernel's exit notification via `exit_watch`. We never
    ///    synthesise an `ExitReport` while the real reap is
    ///    outstanding — a fake "succeeded" report after SIGKILL would
    ///    mislead the supervisor into believing the child is gone when
    ///    it may still hold sockets.
    ///
    /// Consumes `self` so the manager cannot accidentally keep a
    /// half-dead plugin around.
    pub async fn shutdown_gracefully(mut self) -> ExitReport {
        // Best-effort RPC. A buggy plugin might drop the frame — that's
        // OK, the escalation path handles it. §11.1 pins this at 10s
        // independent of `shutdown_grace_ms`.
        let _ = self
            .client
            .call_value_with_timeout(
                "shutdown",
                serde_json::json!({}),
                Some(SHUTDOWN_RPC_TIMEOUT),
            )
            .await;
        self.client.close_writer();

        // §6.3 exit-drain window. Starts fresh after the RPC returns
        // so a fast-answering plugin still gets the full wait.
        if let Some(report) = self.wait_with_grace(self.shutdown_grace).await {
            return report;
        }
        warn!(plugin = %self.plugin_id, "shutdown RPC didn't drain; SIGTERM");

        self.child_kill.signal(libc_term());
        if let Some(report) = self.wait_with_grace(SIGTERM_GRACE).await {
            return report;
        }
        warn!(plugin = %self.plugin_id, "SIGTERM didn't drain; SIGKILL");

        self.child_kill.signal(libc_kill());
        // SIGKILL is non-maskable; the kernel will reap shortly. Wait
        // for the real exit signal. If it genuinely never arrives (which
        // implies either a kernel bug or that the exit watcher task was
        // cancelled before us) we log loudly and return a clearly-marked
        // synthetic failure so the supervisor can still make progress,
        // but we refuse to pretend success.
        match self.wait_with_grace(SIGKILL_REAP_BUDGET).await {
            Some(report) => report,
            None => {
                warn!(
                    plugin = %self.plugin_id,
                    budget_ms = %SIGKILL_REAP_BUDGET.as_millis(),
                    "SIGKILL was not observed within reap budget; synthesising failure report"
                );
                synthetic_post_sigkill_report()
            }
        }
    }

    async fn wait_with_grace(&mut self, grace: Duration) -> Option<ExitReport> {
        // Clone the watch so a prior taker of `exit_rx` doesn't starve
        // us. `changed().await` is error on a closed sender, which
        // means the spawn task panicked — treat as exit.
        let mut rx = self.exit_watch.clone();
        let fut = async {
            loop {
                if let Some(report) = rx.borrow().clone() {
                    return report;
                }
                if rx.changed().await.is_err() {
                    return ExitReport {
                        code: None,
                        signal: None,
                        success: false,
                    };
                }
            }
        };
        tokio::time::timeout(grace, fut).await.ok()
    }
}

impl Drop for SubprocessPlugin {
    fn drop(&mut self) {
        // `kill_on_drop(true)` on the `Command` would already handle
        // this, but the Child was moved into the exit task. Send the
        // signal explicitly to avoid a zombie on abort paths — unless
        // the exit task has already observed reap and disarmed us, in
        // which case the PID may have been recycled and sending would
        // risk hitting an unrelated process. `ChildKill::signal` also
        // checks the disarm flag atomically for the same reason.
        if self.exit_watch.borrow().is_some() {
            debug!(plugin = %self.plugin_id, "SubprocessPlugin dropped after reap; skipping SIGKILL");
            return;
        }
        debug!(plugin = %self.plugin_id, "SubprocessPlugin dropped before reap; sending SIGKILL as safety net");
        self.child_kill.signal(libc_kill());
    }
}

#[cfg(unix)]
fn libc_term() -> i32 {
    libc::SIGTERM
}
#[cfg(unix)]
fn libc_kill() -> i32 {
    libc::SIGKILL
}
#[cfg(not(unix))]
fn libc_term() -> i32 {
    0
}
#[cfg(not(unix))]
fn libc_kill() -> i32 {
    0
}

/// What we return from `shutdown_gracefully` when even `SIGKILL`'s reap
/// notification misses the budget. Extracted so tests can pin the
/// "never claim success" invariant without racing a live child.
fn synthetic_post_sigkill_report() -> ExitReport {
    ExitReport {
        code: None,
        signal: Some(libc_kill()),
        success: false,
    }
}

async fn drain_stderr(
    stderr: tokio::process::ChildStderr,
    plugin_id: String,
    redactions: BTreeSet<String>,
) {
    let mut lines = BufReader::new(stderr).lines();
    let mut malformed_already_warned = false;
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => emit_stderr_line(
                &plugin_id,
                &line,
                &redactions,
                &mut malformed_already_warned,
            ),
            Ok(None) => return,
            Err(err) => {
                warn!(plugin = %plugin_id, error = %err, "plugin stderr read failed; giving up");
                return;
            }
        }
    }
}

/// Pure parse + redact of one stderr line. Separated from tracing
/// emission so it can be unit-tested without installing a subscriber.
///
/// Returns `Some(parsed)` on a well-formed JSON object line with an
/// object-shaped `fields` (or no `fields` at all); `None` when the line
/// is not JSON, is not an object, or carries a non-object `fields`
/// (§8.4 requires object-shaped fields so the host can redact).
///
/// Redaction: every key that appears in `redactions` is rewritten to
/// `"<redacted>"` **at any nesting depth** of the remaining map. This
/// matches §8.2's allowance for nested config objects — a secret field
/// inside `fields.credentials.token` must still be scrubbed.
#[derive(Debug, PartialEq)]
struct StderrRecord {
    level: String,
    message: String,
    fields_json: String,
}

fn redact_in_place(value: &mut Value, redactions: &BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if redactions.contains(k) {
                    *v = Value::String("<redacted>".to_owned());
                } else {
                    redact_in_place(v, redactions);
                }
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                redact_in_place(v, redactions);
            }
        }
        _ => {}
    }
}

fn parse_and_redact_stderr_line(line: &str, redactions: &BTreeSet<String>) -> Option<StderrRecord> {
    let trimmed = line.trim_end();
    if trimmed.is_empty() {
        return None;
    }
    let Ok(Value::Object(mut map)) = serde_json::from_str::<Value>(trimmed) else {
        return None;
    };
    // §8.4 requires the `fields` slot, if present, to be a JSON object.
    // A non-object `fields` would cause us to skip redaction entirely,
    // which is the exact failure mode that leaks secrets. Treat it as
    // malformed and let the caller route through the warning path.
    if let Some(v) = map.get("fields")
        && !v.is_object()
    {
        return None;
    }
    let level = map
        .remove("level")
        .and_then(|v| v.as_str().map(str::to_ascii_lowercase))
        .unwrap_or_else(|| "info".to_owned());
    let message = map
        .remove("message")
        .map(|v| match v {
            Value::String(s) => s,
            other => other.to_string(),
        })
        .unwrap_or_default();
    // Redact across the entire remaining payload, not just the top
    // level of `fields`. A plugin that puts secrets as a sibling of
    // `fields` still gets scrubbed; nested objects inside `fields` are
    // covered; arrays of objects under `fields` are covered.
    for v in map.values_mut() {
        redact_in_place(v, redactions);
    }
    let fields_json = map
        .get("fields")
        .map(ToString::to_string)
        .unwrap_or_else(|| "{}".to_owned());
    Some(StderrRecord {
        level,
        message,
        fields_json,
    })
}

/// Outcome of classifying a stderr line that failed the §8.4 shape
/// check. Extracted as a pure enum so tests can pin the "secrets never
/// leak out of a malformed JSON body" invariant without needing a
/// tracing subscriber.
#[derive(Debug, PartialEq, Eq)]
enum NoncompliantLine {
    /// Parseable JSON, wrong shape. The body MUST be dropped: key-scoped
    /// redaction cannot scrub arbitrary nested string leaves (e.g.
    /// `"fields":"token=leak"`), so logging the body verbatim — even
    /// after running `redact_in_place` — could leak secrets. `shape` is
    /// a values-free summary that is always safe to log.
    ParseableJsonWrongShape { shape: String },
    /// Not JSON at all. Log raw. Plugin authors are responsible for not
    /// piping secrets through unstructured stderr (§8.4).
    NonJson,
}

fn classify_noncompliant_line(trimmed: &str) -> NoncompliantLine {
    match serde_json::from_str::<Value>(trimmed) {
        Ok(val) => NoncompliantLine::ParseableJsonWrongShape {
            shape: describe_shape(&val),
        },
        Err(_) => NoncompliantLine::NonJson,
    }
}

/// Values-free shape description. For objects we emit sorted key names
/// only — never values. Keys can in principle carry information, but in
/// practice plugin authors use stable identifiers here, and having the
/// key set is essential for debugging a schema drift. Values are the
/// actual secret-carrying surface, and are always suppressed.
fn describe_shape(val: &Value) -> String {
    match val {
        Value::Object(map) => {
            let mut keys: Vec<&str> = map.keys().map(|k| k.as_str()).collect();
            keys.sort();
            format!("object(keys=[{}])", keys.join(","))
        }
        Value::Array(arr) => format!("array(len={})", arr.len()),
        Value::String(_) => "string".to_owned(),
        Value::Number(_) => "number".to_owned(),
        Value::Bool(_) => "bool".to_owned(),
        Value::Null => "null".to_owned(),
    }
}

/// Route a single stderr line through the §8.4 redaction path.
///
/// - **Spec-compliant line** (`{ "level", "message", "fields": { .. } }`):
///   `tracing` event at the matching level, with any redaction key
///   scrubbed recursively across the remaining payload.
/// - **Parseable JSON but not §8.4-shaped** (e.g. `fields` is a string,
///   or the top level is an array): drop the body and log only the
///   key-set / value-type summary. Codex pass 4 called out that running
///   key-scoped redaction over the body cannot scrub string-valued
///   leaves like `"fields":"token=leak"` — the secret sits inside a
///   non-secret key, so the recursive walker leaves it intact.
/// - **Non-JSON line**: there is no structured form to redact against,
///   so we log the raw line at `info`. The one-shot advisory still
///   fires so operators notice the non-compliance.
fn emit_stderr_line(
    plugin_id: &str,
    line: &str,
    redactions: &BTreeSet<String>,
    malformed_already_warned: &mut bool,
) {
    let trimmed = line.trim_end();
    if trimmed.is_empty() {
        return;
    }

    if let Some(rec) = parse_and_redact_stderr_line(trimmed, redactions) {
        let StderrRecord {
            level,
            message,
            fields_json,
        } = rec;
        match level.as_str() {
            "error" => error!(plugin = %plugin_id, fields = %fields_json, "{message}"),
            "warn" | "warning" => {
                warn!(plugin = %plugin_id, fields = %fields_json, "{message}")
            }
            "debug" => debug!(plugin = %plugin_id, fields = %fields_json, "{message}"),
            "trace" => {
                debug!(plugin = %plugin_id, level = "trace", fields = %fields_json, "{message}")
            }
            _ => info!(plugin = %plugin_id, fields = %fields_json, "{message}"),
        }
        return;
    }

    if !*malformed_already_warned {
        warn!(
            plugin = %plugin_id,
            "plugin stderr is not §8.4-compliant (JSON object per line with object-shaped `fields`); dropping bodies to prevent secret leak"
        );
        *malformed_already_warned = true;
    }
    match classify_noncompliant_line(trimmed) {
        NoncompliantLine::ParseableJsonWrongShape { shape } => {
            info!(
                plugin = %plugin_id,
                shape = %shape,
                "stderr (non-compliant JSON; body dropped)"
            );
        }
        NoncompliantLine::NonJson => {
            info!(plugin = %plugin_id, "stderr: {trimmed}");
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
