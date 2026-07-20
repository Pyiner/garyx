use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::discovery::discover_conversation_id;
use crate::error::{AntigravityError, Result};
use crate::transcript::{TranscriptDecoder, max_step_index, transcript_path};
use crate::types::{
    AntigravityEvent, AntigravityRunFailure, AntigravityRunFailureKind, AntigravityRunOutcome,
    AntigravityRunRequest, ApprovalDecision, ApprovalRequest,
};

const OUTER_TIMEOUT_GRACE: Duration = Duration::from_secs(10);
const FINAL_TRANSCRIPT_POLLS: usize = 3;
const FINAL_TRANSCRIPT_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone)]
pub struct AntigravityClientConfig {
    pub cli_bin: String,
    pub brain_root: PathBuf,
    pub transcript_poll_interval: Duration,
    pub discovery_timeout: Duration,
    pub shutdown_grace: Duration,
    pub run_timeout_grace: Duration,
}

impl AntigravityClientConfig {
    pub fn new(cli_bin: impl Into<String>, brain_root: impl Into<PathBuf>) -> Self {
        Self {
            cli_bin: cli_bin.into(),
            brain_root: brain_root.into(),
            transcript_poll_interval: Duration::from_millis(250),
            discovery_timeout: Duration::from_secs(30),
            shutdown_grace: Duration::from_secs(2),
            run_timeout_grace: OUTER_TIMEOUT_GRACE,
        }
    }
}

/// Process owner and transcript transport for Antigravity CLI runs.
pub struct AntigravityClient {
    config: AntigravityClientConfig,
    active_runs: Mutex<HashMap<String, Arc<Mutex<Child>>>>,
    fresh_session_lock: Mutex<()>,
}

impl AntigravityClient {
    pub fn new(config: AntigravityClientConfig) -> Self {
        Self {
            config,
            active_runs: Mutex::new(HashMap::new()),
            fresh_session_lock: Mutex::new(()),
        }
    }

    /// Invoke the CLI's read-only model listing as an availability probe.
    pub async fn probe(&self) -> Result<()> {
        let output = Command::new(&self.config.cli_bin)
            .arg("models")
            .output()
            .await
            .map_err(|error| AntigravityError::Spawn(error.to_string()))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(AntigravityError::NotReady(append_process_output(
                format!("models exited with status {}", output.status),
                &String::from_utf8_lossy(&output.stdout),
                &String::from_utf8_lossy(&output.stderr),
            )))
        }
    }

    pub async fn execute(
        &self,
        request: AntigravityRunRequest,
        on_event: &(dyn Fn(AntigravityEvent) + Send + Sync),
    ) -> Result<AntigravityRunOutcome> {
        let run_id = request.run_id.clone();
        let deadline = request
            .print_timeout
            .saturating_add(self.config.run_timeout_grace);
        match tokio::time::timeout(deadline, self.run_once(request, on_event)).await {
            Ok(result) => result,
            Err(_) => {
                let _ = self.abort(&run_id).await;
                Err(AntigravityError::Timeout)
            }
        }
    }

    async fn run_once(
        &self,
        request: AntigravityRunRequest,
        on_event: &(dyn Fn(AntigravityEvent) + Send + Sync),
    ) -> Result<AntigravityRunOutcome> {
        let approval = (request.approval_callback)(ApprovalRequest {
            model: request.model.clone(),
            conversation_id: request.conversation_id.clone(),
            workspace_dir: request.workspace_dir.clone(),
        })
        .await?;
        if let ApprovalDecision::Deny { reason } = approval {
            return Err(AntigravityError::ApprovalDenied(reason));
        }

        let transcript_baseline = request
            .conversation_id
            .as_deref()
            .map(|id| max_step_index(&transcript_path(&self.config.brain_root, id)))
            .unwrap_or(-1);
        let fresh_guard = if request.conversation_id.is_none() {
            Some(self.fresh_session_lock.lock().await)
        } else {
            None
        };
        let run_start = SystemTime::now();
        let args = build_command_args(
            &request.prompt,
            &request.model,
            request.conversation_id.as_deref(),
            &request.workspace_dir,
            &request.log_path,
            request.print_timeout,
            &approval,
        );
        let mut command = Command::new(&self.config.cli_bin);
        command.args(&args);
        command.current_dir(&request.workspace_dir);
        command.stdin(std::process::Stdio::null());
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());
        command.kill_on_drop(true);
        command.envs(&request.env);

        let mut child = command
            .spawn()
            .map_err(|error| AntigravityError::Spawn(error.to_string()))?;
        let stdout = child.stdout.take().ok_or_else(|| {
            AntigravityError::Transport("antigravity stdout unavailable".to_owned())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            AntigravityError::Transport("antigravity stderr unavailable".to_owned())
        })?;
        let stdout_task = tokio::spawn(read_stream_to_string(stdout));
        let stderr_task = tokio::spawn(read_stream_to_string(stderr));
        let child = Arc::new(Mutex::new(child));
        self.register_run(&request.run_id, Arc::clone(&child)).await;

        let conversation_id = if let Some(conversation_id) = request.conversation_id.clone() {
            conversation_id
        } else {
            match discover_conversation_id(
                &request.log_path,
                &self.config.brain_root,
                run_start,
                &request.prompt,
                &request.discovery_text,
                self.config.discovery_timeout,
                self.config.transcript_poll_interval,
            )
            .await
            {
                Some(id) => id,
                None => {
                    let (stdout_output, stderr_output) = self
                        .cleanup_registered_run(&request.run_id, stdout_task, stderr_task, true)
                        .await;
                    return Err(AntigravityError::DiscoveryTimeout(process_output_suffix(
                        &stdout_output,
                        &stderr_output,
                    )));
                }
            }
        };
        drop(fresh_guard);
        on_event(AntigravityEvent::SessionBound {
            conversation_id: conversation_id.clone(),
        });

        let transcript = transcript_path(&self.config.brain_root, &conversation_id);
        let started = Instant::now();
        let mut decoder = TranscriptDecoder::new();
        let exit_status = loop {
            decoder.apply_path(&transcript, transcript_baseline, on_event);
            let maybe_status = {
                let mut child = child.lock().await;
                child.try_wait()
            };
            match maybe_status {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    tokio::time::sleep(self.config.transcript_poll_interval).await;
                }
                Err(error) => {
                    let _ = self
                        .cleanup_registered_run(&request.run_id, stdout_task, stderr_task, true)
                        .await;
                    return Err(AntigravityError::ProcessWait(error.to_string()));
                }
            }
        };

        for _ in 0..FINAL_TRANSCRIPT_POLLS {
            decoder.apply_path(&transcript, transcript_baseline, on_event);
            tokio::time::sleep(FINAL_TRANSCRIPT_POLL_INTERVAL).await;
        }
        let duration = started.elapsed();

        let (stdout_output, stderr_output) = self
            .cleanup_registered_run(&request.run_id, stdout_task, stderr_task, false)
            .await;

        classify_outcome(
            conversation_id,
            exit_status,
            decoder.last_error(),
            decoder.has_visible_output(),
            &stdout_output,
            &stderr_output,
            duration,
        )
    }

    async fn register_run(&self, run_id: &str, child: Arc<Mutex<Child>>) {
        self.active_runs
            .lock()
            .await
            .insert(run_id.to_owned(), child);
    }

    async fn unregister_run(&self, run_id: &str) -> Option<Arc<Mutex<Child>>> {
        self.active_runs.lock().await.remove(run_id)
    }

    async fn cleanup_registered_run(
        &self,
        run_id: &str,
        stdout_task: JoinHandle<String>,
        stderr_task: JoinHandle<String>,
        kill_child: bool,
    ) -> (String, String) {
        let child = self.unregister_run(run_id).await;
        self.cleanup_run_io(child, stdout_task, stderr_task, kill_child)
            .await
    }

    async fn cleanup_run_io(
        &self,
        child: Option<Arc<Mutex<Child>>>,
        stdout_task: JoinHandle<String>,
        stderr_task: JoinHandle<String>,
        kill_child: bool,
    ) -> (String, String) {
        tokio::time::timeout(self.config.shutdown_grace, async move {
            if let Some(child) = child {
                let mut child = child.lock().await;
                if kill_child {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                }
            }
            let stdout = stdout_task.await.unwrap_or_default();
            let stderr = stderr_task.await.unwrap_or_default();
            (stdout, stderr)
        })
        .await
        .unwrap_or_default()
    }

    /// Abort and reap one active run.
    pub async fn abort(&self, run_id: &str) -> bool {
        let Some(child) = self.unregister_run(run_id).await else {
            return false;
        };
        let mut child = child.lock().await;
        let _ = child.kill().await;
        let _ = child.wait().await;
        true
    }

    /// Abort and reap every active run.
    pub async fn shutdown(&self) {
        let run_ids = self
            .active_runs
            .lock()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for run_id in run_ids {
            let _ = self.abort(&run_id).await;
        }
    }
}

fn build_command_args(
    prompt: &str,
    model: &str,
    conversation_id: Option<&str>,
    workspace_dir: &Path,
    log_path: &Path,
    timeout: Duration,
    approval: &ApprovalDecision,
) -> Vec<String> {
    let mut args = vec![
        "-p".to_owned(),
        prompt.to_owned(),
        "--model".to_owned(),
        model.to_owned(),
    ];
    match approval {
        ApprovalDecision::UseCliDefault => {}
        ApprovalDecision::AcceptEdits => {
            args.push("--mode".to_owned());
            args.push("accept-edits".to_owned());
        }
        ApprovalDecision::Plan => {
            args.push("--mode".to_owned());
            args.push("plan".to_owned());
        }
        ApprovalDecision::BypassPermissions => {
            args.push("--dangerously-skip-permissions".to_owned());
        }
        ApprovalDecision::Deny { .. } => {
            debug_assert!(false, "denied runs must not build a command");
        }
    }
    args.push("--print-timeout".to_owned());
    args.push(print_timeout_arg(timeout));
    args.push("--log-file".to_owned());
    args.push(log_path.to_string_lossy().into_owned());
    if let Some(conversation_id) = conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push("--conversation".to_owned());
        args.push(conversation_id.to_owned());
    }
    args.push("--add-dir".to_owned());
    args.push(workspace_dir.to_string_lossy().into_owned());
    args
}

fn print_timeout_arg(timeout: Duration) -> String {
    format!("{}s", timeout.as_secs().max(1))
}

async fn read_stream_to_string<T>(stream: T) -> String
where
    T: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(stream).lines();
    let mut output = Vec::new();
    while let Ok(Some(line)) = reader.next_line().await {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            output.push(trimmed.to_owned());
        }
    }
    output.join("\n")
}

fn invalid_conversation_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("session not found")
        || lower.contains("invalid session")
        || lower.contains("conversation not found")
}

fn classify_failure(
    message: String,
    default_kind: AntigravityRunFailureKind,
) -> AntigravityRunFailure {
    let kind = if invalid_conversation_message(&message) {
        AntigravityRunFailureKind::InvalidConversation
    } else {
        default_kind
    };
    AntigravityRunFailure { kind, message }
}

fn classify_outcome(
    conversation_id: String,
    exit_status: ExitStatus,
    transcript_error: Option<&str>,
    visible_output: bool,
    stdout_output: &str,
    stderr_output: &str,
    duration: Duration,
) -> Result<AntigravityRunOutcome> {
    if !exit_status.success() {
        let process_error = append_process_output(
            format!("antigravity CLI exited with status {exit_status}"),
            stdout_output,
            stderr_output,
        );
        if !visible_output {
            return if invalid_conversation_message(&process_error) {
                Err(AntigravityError::InvalidConversation(process_error))
            } else {
                Err(AntigravityError::ProcessExited(process_error))
            };
        }
        let failure = transcript_error.map_or_else(
            || classify_failure(process_error, AntigravityRunFailureKind::ProcessExit),
            |error| classify_failure(error.to_owned(), AntigravityRunFailureKind::Transcript),
        );
        return Ok(AntigravityRunOutcome {
            conversation_id,
            success: false,
            failure: Some(failure),
            duration,
        });
    }

    let failure = transcript_error
        .map(|error| classify_failure(error.to_owned(), AntigravityRunFailureKind::Transcript));
    Ok(AntigravityRunOutcome {
        conversation_id,
        success: failure.is_none(),
        failure,
        duration,
    })
}

fn process_output_suffix(stdout_output: &str, stderr_output: &str) -> String {
    append_process_output(String::new(), stdout_output, stderr_output)
}

fn append_process_output(
    message: impl Into<String>,
    stdout_output: &str,
    stderr_output: &str,
) -> String {
    let mut message = message.into();
    let stdout_output = stdout_output.trim();
    let stderr_output = stderr_output.trim();
    if !stderr_output.is_empty() {
        message.push_str(" | stderr: ");
        message.push_str(stderr_output);
    }
    if !stdout_output.is_empty() {
        message.push_str(" | stdout: ");
        message.push_str(stdout_output);
    }
    message
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc as StdArc, Mutex as StdMutex};

    use super::*;
    use crate::types::{ApprovalCallback, ApprovalFuture};

    fn approval(decision: ApprovalDecision) -> ApprovalCallback {
        StdArc::new(move |_| {
            let decision = decision.clone();
            Box::pin(async move { Ok(decision) }) as ApprovalFuture
        })
    }

    fn test_request(
        workspace_dir: &Path,
        log_path: &Path,
        decision: ApprovalDecision,
    ) -> AntigravityRunRequest {
        AntigravityRunRequest {
            run_id: "synthetic-run".to_owned(),
            prompt: "synthetic prompt".to_owned(),
            discovery_text: "synthetic prompt".to_owned(),
            model: "Synthetic Model".to_owned(),
            conversation_id: Some("synthetic-session".to_owned()),
            workspace_dir: workspace_dir.to_path_buf(),
            log_path: log_path.to_path_buf(),
            env: HashMap::new(),
            print_timeout: Duration::from_secs(2),
            approval_callback: approval(decision),
        }
    }

    #[test]
    fn command_args_encode_only_the_callers_approval_decision() {
        let workspace = Path::new("/tmp/synthetic-workspace");
        let log = Path::new("/tmp/synthetic-antigravity.log");
        for (decision, expected) in [
            (ApprovalDecision::UseCliDefault, Vec::<&str>::new()),
            (
                ApprovalDecision::AcceptEdits,
                vec!["--mode", "accept-edits"],
            ),
            (ApprovalDecision::Plan, vec!["--mode", "plan"]),
            (
                ApprovalDecision::BypassPermissions,
                vec!["--dangerously-skip-permissions"],
            ),
        ] {
            let args = build_command_args(
                "hello",
                "Synthetic Model",
                Some("synthetic-session"),
                workspace,
                log,
                Duration::from_secs(12),
                &decision,
            );
            assert_eq!(
                args.contains(&"--dangerously-skip-permissions".to_owned()),
                decision == ApprovalDecision::BypassPermissions
            );
            for value in expected {
                assert!(
                    args.contains(&value.to_owned()),
                    "missing {value}: {args:?}"
                );
            }
            assert!(args.contains(&"--print-timeout".to_owned()));
            assert!(args.contains(&"12s".to_owned()));
        }
    }

    #[tokio::test]
    async fn denied_approval_prevents_process_spawn() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config = AntigravityClientConfig::new(
            temp.path().join("does-not-exist").to_string_lossy(),
            temp.path().join("brain"),
        );
        let client = AntigravityClient::new(config);
        let request = test_request(
            temp.path(),
            &temp.path().join("run.log"),
            ApprovalDecision::Deny {
                reason: "synthetic denial".to_owned(),
            },
        );

        let error = client.execute(request, &|_| {}).await.expect_err("denied");
        assert!(matches!(error, AntigravityError::ApprovalDenied(_)));
    }

    #[tokio::test]
    async fn approval_callback_failure_prevents_process_spawn() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config = AntigravityClientConfig::new(
            temp.path().join("does-not-exist").to_string_lossy(),
            temp.path().join("brain"),
        );
        let client = AntigravityClient::new(config);
        let mut request = test_request(
            temp.path(),
            &temp.path().join("run.log"),
            ApprovalDecision::UseCliDefault,
        );
        request.approval_callback = StdArc::new(|_| {
            Box::pin(async {
                Err(AntigravityError::Transport(
                    "synthetic policy failure".to_owned(),
                ))
            }) as ApprovalFuture
        });

        let error = client
            .execute(request, &|_| {})
            .await
            .expect_err("callback failure");
        assert!(matches!(error, AntigravityError::Transport(_)));
    }

    struct FakeCliFixture {
        _temp: tempfile::TempDir,
        workspace: PathBuf,
        brain: PathBuf,
        script: PathBuf,
        log: PathBuf,
    }

    impl FakeCliFixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().expect("tempdir");
            let workspace = temp.path().join("workspace");
            let brain = temp
                .path()
                .join(".gemini")
                .join("antigravity-cli")
                .join("brain");
            fs::create_dir_all(&workspace).expect("workspace");
            fs::create_dir_all(&brain).expect("brain");
            let script = temp.path().join("fake-agy.py");
            fs::write(
                &script,
                r#"#!/usr/bin/env python3
import json
import os
import sys
import time

if len(sys.argv) > 1 and sys.argv[1] == "models":
    print("Synthetic Model")
    sys.exit(0)

brain = os.environ["FAKE_AGY_BRAIN_ROOT"]
mode = os.environ.get("FAKE_AGY_MODE", "success")
identity = os.environ.get("SYNTHETIC_RUN_IDENTITY", "missing")
conversation = None
prompt = ""
log_path = None
for index, arg in enumerate(sys.argv):
    if arg == "--conversation":
        conversation = sys.argv[index + 1]
    elif arg == "-p":
        prompt = sys.argv[index + 1]
    elif arg == "--log-file":
        log_path = sys.argv[index + 1]
if not conversation:
    conversation = os.environ.get("FAKE_AGY_SESSION", "synthetic-fresh-session")

base = os.path.dirname(brain)
os.makedirs(os.path.join(base, "conversations"), exist_ok=True)
open(os.path.join(base, "conversations", conversation + ".db"), "a").close()
logs = os.path.join(brain, conversation, ".system_generated", "logs")
os.makedirs(logs, exist_ok=True)
path = os.path.join(logs, "transcript.jsonl")
with open(path, "a") as transcript:
    existing = sum(1 for _ in open(path)) if os.path.getsize(path) else 0
    step = existing + 1
    transcript.write(json.dumps({"type":"USER_INPUT", "step_index":step, "content":prompt}) + "\n")
    step += 1
    if mode in ("success", "visible_exit", "visible_error_exit", "slow"):
        transcript.write(json.dumps({"type":"PLANNER_RESPONSE", "step_index":step, "content":identity}) + "\n")
        step += 1
    if mode in ("clean_error", "visible_error_exit", "error_exit"):
        transcript.write(json.dumps({"type":"ERROR_MESSAGE", "step_index":step, "error":"synthetic transcript error"}) + "\n")
    transcript.flush()
if log_path:
    with open(log_path, "a") as log:
        log.write(conversation + "\n")
if mode == "slow":
    time.sleep(30)
if mode in ("visible_exit", "visible_error_exit", "error_exit"):
    print("synthetic process diagnostic", file=sys.stderr)
    sys.exit(7)
sys.exit(0)
"#,
            )
            .expect("script");
            let mut permissions = fs::metadata(&script).expect("metadata").permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&script, permissions).expect("chmod");
            let log = temp.path().join("run.log");
            Self {
                _temp: temp,
                workspace,
                brain,
                script,
                log,
            }
        }

        fn client(&self) -> AntigravityClient {
            AntigravityClient::new(AntigravityClientConfig::new(
                self.script.to_string_lossy(),
                self.brain.clone(),
            ))
        }

        fn request(
            &self,
            run_id: &str,
            session: Option<&str>,
            mode: &str,
            identity: &str,
        ) -> AntigravityRunRequest {
            let mut env = HashMap::from([
                (
                    "FAKE_AGY_BRAIN_ROOT".to_owned(),
                    self.brain.to_string_lossy().into_owned(),
                ),
                ("FAKE_AGY_MODE".to_owned(), mode.to_owned()),
                ("SYNTHETIC_RUN_IDENTITY".to_owned(), identity.to_owned()),
            ]);
            if let Some(session) = session {
                env.insert("FAKE_AGY_SESSION".to_owned(), session.to_owned());
            }
            AntigravityRunRequest {
                run_id: run_id.to_owned(),
                prompt: format!("prompt for {run_id}"),
                discovery_text: format!("prompt for {run_id}"),
                model: "Synthetic Model".to_owned(),
                conversation_id: session.map(ToOwned::to_owned),
                workspace_dir: self.workspace.clone(),
                log_path: self.log.clone(),
                env,
                print_timeout: Duration::from_secs(2),
                approval_callback: approval(ApprovalDecision::BypassPermissions),
            }
        }
    }

    #[tokio::test]
    async fn fake_cli_tails_fresh_transcript_and_preserves_per_run_env() {
        let fixture = FakeCliFixture::new();
        let client = fixture.client();
        client.probe().await.expect("probe");

        let fresh_events = StdArc::new(StdMutex::new(Vec::new()));
        let events_for_callback = StdArc::clone(&fresh_events);
        let callback = move |event| events_for_callback.lock().unwrap().push(event);
        let outcome = client
            .execute(
                fixture.request("synthetic-fresh-run", None, "success", "synthetic-fresh"),
                &callback,
            )
            .await
            .expect("fresh run");
        assert_eq!(outcome.conversation_id, "synthetic-fresh-session");
        assert!(fresh_events.lock().unwrap().iter().any(|event| {
            matches!(event, AntigravityEvent::SessionBound { conversation_id }
                if conversation_id == "synthetic-fresh-session")
        }));

        for (index, identity) in ["synthetic-one", "synthetic-two"].into_iter().enumerate() {
            let session = format!("synthetic-env-session-{index}");
            let events = StdArc::new(StdMutex::new(Vec::new()));
            let events_for_callback = StdArc::clone(&events);
            let callback = move |event| events_for_callback.lock().unwrap().push(event);
            let outcome = client
                .execute(
                    fixture.request(
                        &format!("synthetic-env-run-{index}"),
                        Some(&session),
                        "success",
                        identity,
                    ),
                    &callback,
                )
                .await
                .expect("run");
            assert!(outcome.success);
            assert!(events.lock().unwrap().iter().any(|event| {
                matches!(event, AntigravityEvent::AssistantDelta { text, .. } if text == identity)
            }));
        }
    }

    #[tokio::test]
    async fn exit_and_transcript_error_matrix_matches_legacy_adapter() {
        let fixture = FakeCliFixture::new();
        let client = fixture.client();

        let success = client
            .execute(
                fixture.request("success-run", Some("success-session"), "success", "ok"),
                &|_| {},
            )
            .await
            .expect("success");
        assert!(success.success);
        assert!(success.failure.is_none());

        let clean_error = client
            .execute(
                fixture.request(
                    "clean-error-run",
                    Some("clean-error-session"),
                    "clean_error",
                    "ignored",
                ),
                &|_| {},
            )
            .await
            .expect("clean error outcome");
        assert!(!clean_error.success);
        assert_eq!(
            clean_error
                .failure
                .as_ref()
                .map(|failure| failure.message.as_str()),
            Some("synthetic transcript error")
        );

        let visible_exit = client
            .execute(
                fixture.request(
                    "visible-exit-run",
                    Some("visible-exit-session"),
                    "visible_exit",
                    "partial",
                ),
                &|_| {},
            )
            .await
            .expect("partial outcome");
        assert!(!visible_exit.success);
        let visible_failure = visible_exit.failure.expect("failure");
        assert_eq!(visible_failure.kind, AntigravityRunFailureKind::ProcessExit);
        assert!(
            visible_failure
                .message
                .contains("synthetic process diagnostic")
        );

        let visible_error_exit = client
            .execute(
                fixture.request(
                    "visible-error-run",
                    Some("visible-error-session"),
                    "visible_error_exit",
                    "partial",
                ),
                &|_| {},
            )
            .await
            .expect("partial transcript error outcome");
        assert_eq!(
            visible_error_exit
                .failure
                .as_ref()
                .map(|failure| failure.message.as_str()),
            Some("synthetic transcript error")
        );

        let error_only = client
            .execute(
                fixture.request(
                    "error-only-run",
                    Some("error-only-session"),
                    "error_exit",
                    "ignored",
                ),
                &|_| {},
            )
            .await
            .expect_err("hard process error");
        assert!(matches!(error_only, AntigravityError::ProcessExited(_)));
        assert!(
            error_only
                .to_string()
                .contains("synthetic process diagnostic")
        );
    }

    #[tokio::test]
    async fn abort_kills_and_reaps_registered_child() {
        let fixture = FakeCliFixture::new();
        let client = StdArc::new(fixture.client());
        let run_id = "synthetic-slow-run";
        let request = fixture.request(run_id, Some("synthetic-slow-session"), "slow", "partial");
        let client_for_run = StdArc::clone(&client);
        let run = tokio::spawn(async move { client_for_run.execute(request, &|_| {}).await });

        let mut aborted = false;
        for _ in 0..40 {
            if client.abort(run_id).await {
                aborted = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(aborted, "run never registered");
        let result = tokio::time::timeout(Duration::from_secs(3), run)
            .await
            .expect("run stopped")
            .expect("join");
        assert!(result.is_ok() || matches!(result, Err(AntigravityError::ProcessExited(_))));
    }

    #[tokio::test]
    async fn timeout_aborts_child_and_clears_run_registry() {
        let fixture = FakeCliFixture::new();
        let mut config =
            AntigravityClientConfig::new(fixture.script.to_string_lossy(), fixture.brain.clone());
        config.run_timeout_grace = Duration::from_millis(300);
        let client = AntigravityClient::new(config);
        let mut request = fixture.request(
            "synthetic-timeout-run",
            Some("synthetic-timeout-session"),
            "slow",
            "partial",
        );
        request.print_timeout = Duration::ZERO;

        let error = client.execute(request, &|_| {}).await.expect_err("timeout");
        assert!(matches!(error, AntigravityError::Timeout));
        assert!(!client.abort("synthetic-timeout-run").await);
    }

    #[tokio::test]
    async fn shutdown_aborts_all_registered_children() {
        let fixture = FakeCliFixture::new();
        let client = StdArc::new(fixture.client());
        let bound_count = StdArc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for index in 0..2 {
            let run_id = format!("synthetic-shutdown-run-{index}");
            let session_id = format!("synthetic-shutdown-session-{index}");
            let request = fixture.request(
                &run_id,
                Some(&session_id),
                "slow",
                &format!("partial-{index}"),
            );
            let client_for_run = StdArc::clone(&client);
            let count_for_callback = StdArc::clone(&bound_count);
            handles.push(tokio::spawn(async move {
                let callback = move |event| {
                    if matches!(event, AntigravityEvent::SessionBound { .. }) {
                        count_for_callback.fetch_add(1, Ordering::SeqCst);
                    }
                };
                client_for_run.execute(request, &callback).await
            }));
        }

        for _ in 0..80 {
            if bound_count.load(Ordering::SeqCst) == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert_eq!(bound_count.load(Ordering::SeqCst), 2, "runs never bound");
        client.shutdown().await;

        for handle in handles {
            let _ = tokio::time::timeout(Duration::from_secs(3), handle)
                .await
                .expect("run stopped")
                .expect("join");
        }
        assert!(!client.abort("synthetic-shutdown-run-0").await);
        assert!(!client.abort("synthetic-shutdown-run-1").await);
    }
}
