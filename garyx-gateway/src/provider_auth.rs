use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use garyx_models::config::{AgentProviderConfig, GaryxConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::server::AppState;

const AUTH_START_TIMEOUT: Duration = Duration::from_secs(30);
const CCTTY_BINARY_NAME: &str = "cctty";
const EMBEDDED_CCTTY_ARG: &str = "__cctty";
const GARYX_CCTTY_PATH_ENV: &str = "GARYX_CCTTY_PATH";
const GARYX_CLAUDE_CLI_PATH_ENV: &str = "GARYX_CLAUDE_CLI_PATH";
const GARYX_CLAUDE_CLI_MODE_ENV: &str = "GARYX_CLAUDE_CLI_MODE";

#[derive(Default)]
pub struct ClaudeAuthSessionStore {
    sessions: Mutex<HashMap<String, Arc<ClaudeAuthSession>>>,
    command_override: Mutex<Option<AuthCommandSpec>>,
}

impl ClaudeAuthSessionStore {
    async fn insert(&self, session: Arc<ClaudeAuthSession>) {
        self.sessions
            .lock()
            .await
            .insert(session.login_id(), session);
    }

    async fn get(&self, login_id: &str) -> Option<Arc<ClaudeAuthSession>> {
        self.sessions.lock().await.get(login_id).cloned()
    }

    async fn command_override(&self) -> Option<AuthCommandSpec> {
        self.command_override.lock().await.clone()
    }

    #[cfg(test)]
    pub async fn set_command_override_for_test(&self, command: PathBuf) {
        *self.command_override.lock().await = Some(AuthCommandSpec {
            program: command,
            prefix_args: Vec::new(),
        });
    }
}

struct ClaudeAuthSession {
    login_id: String,
    state: Mutex<ClaudeAuthSessionState>,
    stdin: Mutex<Option<ChildStdin>>,
}

impl ClaudeAuthSession {
    fn new(login_id: String, stdin: ChildStdin) -> Self {
        Self {
            login_id,
            state: Mutex::new(ClaudeAuthSessionState {
                status: ClaudeAuthLoginStatus::Starting,
                url: None,
                auth_status: None,
                error: None,
                exit_code: None,
            }),
            stdin: Mutex::new(Some(stdin)),
        }
    }

    fn login_id(&self) -> String {
        self.login_id.clone()
    }

    async fn snapshot(&self) -> ClaudeAuthLoginResponse {
        self.state.lock().await.to_response(&self.login_id)
    }

    async fn update(&self, update: impl FnOnce(&mut ClaudeAuthSessionState)) {
        let mut state = self.state.lock().await;
        update(&mut state);
    }

    async fn submit_code(&self, code: &str) -> Result<ClaudeAuthLoginResponse, ApiError> {
        let mut stdin_guard = self.stdin.lock().await;
        let Some(stdin) = stdin_guard.as_mut() else {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "claude_auth_session_closed",
                "Claude Code auth session is no longer accepting input.",
            ));
        };
        stdin.write_all(code.as_bytes()).await.map_err(|error| {
            ApiError::io(StatusCode::BAD_GATEWAY, "write_auth_code_failed", error)
        })?;
        stdin.write_all(b"\n").await.map_err(|error| {
            ApiError::io(StatusCode::BAD_GATEWAY, "write_auth_code_failed", error)
        })?;
        stdin.flush().await.map_err(|error| {
            ApiError::io(StatusCode::BAD_GATEWAY, "flush_auth_code_failed", error)
        })?;
        drop(stdin_guard);

        self.update(|state| {
            if !state.status.is_terminal() {
                state.status = ClaudeAuthLoginStatus::Submitted;
                state.error = None;
            }
        })
        .await;
        Ok(self.snapshot().await)
    }

    async fn close_stdin(&self) {
        self.stdin.lock().await.take();
    }
}

#[derive(Debug, Clone)]
struct AuthCommandSpec {
    program: PathBuf,
    prefix_args: Vec<String>,
}

#[derive(Debug, Clone)]
struct ClaudeAuthSessionState {
    status: ClaudeAuthLoginStatus,
    url: Option<String>,
    auth_status: Option<Value>,
    error: Option<String>,
    exit_code: Option<i32>,
}

impl ClaudeAuthSessionState {
    fn to_response(&self, login_id: &str) -> ClaudeAuthLoginResponse {
        ClaudeAuthLoginResponse {
            login_id: login_id.to_owned(),
            status: self.status,
            url: self.url.clone(),
            auth_status: self.auth_status.clone(),
            error: self.error.clone(),
            exit_code: self.exit_code,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeAuthLoginStatus {
    Starting,
    WaitingForCode,
    Submitted,
    Succeeded,
    Failed,
}

impl ClaudeAuthLoginStatus {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeAuthLoginMode {
    Claudeai,
    Console,
}

impl Default for ClaudeAuthLoginMode {
    fn default() -> Self {
        Self::Claudeai
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct StartClaudeAuthRequest {
    #[serde(default)]
    mode: ClaudeAuthLoginMode,
    #[serde(default)]
    sso: bool,
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SubmitClaudeAuthRequest {
    code: Option<String>,
    token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaudeAuthLoginResponse {
    pub login_id: String,
    pub status: ClaudeAuthLoginStatus,
    pub url: Option<String>,
    pub auth_status: Option<Value>,
    pub error: Option<String>,
    pub exit_code: Option<i32>,
}

pub async fn start_claude_code_auth(
    State(state): State<Arc<AppState>>,
    Json(request): Json<StartClaudeAuthRequest>,
) -> Result<(StatusCode, Json<ClaudeAuthLoginResponse>), ApiError> {
    let command_spec = resolve_auth_command(&state).await?;
    let login_id = Uuid::new_v4().to_string();
    let login_args = build_login_args(&request);
    let mut command = Command::new(&command_spec.program);
    command.args(&command_spec.prefix_args).args(&login_args);
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut child = command.spawn().map_err(|error| {
        ApiError::io(StatusCode::BAD_GATEWAY, "spawn_claude_auth_failed", error)
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "claude_auth_stdout_unavailable",
            "Claude Code auth subprocess did not expose stdout.",
        )
    })?;
    let stdin = child.stdin.take().ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "claude_auth_stdin_unavailable",
            "Claude Code auth subprocess did not expose stdin.",
        )
    })?;
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(drain_stderr(login_id.clone(), stderr));
    }

    let session = Arc::new(ClaudeAuthSession::new(login_id.clone(), stdin));
    state
        .ops
        .provider_auth_sessions
        .insert(session.clone())
        .await;
    let (started_tx, started_rx) = oneshot::channel();
    tokio::spawn(read_auth_events(
        session.clone(),
        command_spec,
        stdout,
        child,
        started_tx,
    ));

    match tokio::time::timeout(AUTH_START_TIMEOUT, started_rx).await {
        Ok(Ok(Ok(response))) => Ok((StatusCode::CREATED, Json(response))),
        Ok(Ok(Err(error))) => Err(error),
        Ok(Err(_)) => Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "claude_auth_start_interrupted",
            "Claude Code auth session ended before returning a login URL.",
        )),
        Err(_) => {
            session
                .update(|state| {
                    state.status = ClaudeAuthLoginStatus::Failed;
                    state.error = Some("Timed out waiting for Claude Code login URL.".to_owned());
                })
                .await;
            Err(ApiError::new(
                StatusCode::GATEWAY_TIMEOUT,
                "claude_auth_start_timeout",
                "Timed out waiting for Claude Code login URL.",
            ))
        }
    }
}

pub async fn submit_claude_code_auth(
    State(state): State<Arc<AppState>>,
    AxumPath(login_id): AxumPath<String>,
    Json(request): Json<SubmitClaudeAuthRequest>,
) -> Result<Json<ClaudeAuthLoginResponse>, ApiError> {
    let code = request
        .code
        .or(request.token)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "missing_auth_code",
                "Request body must include a non-empty code or token.",
            )
        })?;
    let session = state
        .ops
        .provider_auth_sessions
        .get(&login_id)
        .await
        .ok_or_else(|| unknown_session_error(&login_id))?;
    session.submit_code(&code).await.map(Json)
}

pub async fn get_claude_code_auth(
    State(state): State<Arc<AppState>>,
    AxumPath(login_id): AxumPath<String>,
) -> Result<Json<ClaudeAuthLoginResponse>, ApiError> {
    let session = state
        .ops
        .provider_auth_sessions
        .get(&login_id)
        .await
        .ok_or_else(|| unknown_session_error(&login_id))?;
    Ok(Json(session.snapshot().await))
}

async fn read_auth_events(
    session: Arc<ClaudeAuthSession>,
    command_spec: AuthCommandSpec,
    stdout: tokio::process::ChildStdout,
    mut child: tokio::process::Child,
    started_tx: oneshot::Sender<Result<ClaudeAuthLoginResponse, ApiError>>,
) {
    let mut started_tx = Some(started_tx);
    let mut lines = BufReader::new(stdout).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let Ok(event) = serde_json::from_str::<Value>(&line) else {
                    debug!(line = %line, "ignored non-json Claude auth output");
                    continue;
                };
                handle_auth_event(&session, &command_spec, &event, &mut started_tx).await;
            }
            Ok(None) => break,
            Err(error) => {
                session
                    .update(|state| {
                        if !state.status.is_terminal() {
                            state.status = ClaudeAuthLoginStatus::Failed;
                            state.error =
                                Some(format!("Failed reading Claude Code auth output: {error}"));
                        }
                    })
                    .await;
                send_start_error_once(
                    &mut started_tx,
                    ApiError::io(
                        StatusCode::BAD_GATEWAY,
                        "read_claude_auth_events_failed",
                        error,
                    ),
                );
                break;
            }
        }
    }

    session.close_stdin().await;
    match child.wait().await {
        Ok(status) => {
            let code = status.code();
            let mut should_fetch_status = false;
            session
                .update(|state| {
                    state.exit_code = code;
                    if state.status == ClaudeAuthLoginStatus::Succeeded {
                        should_fetch_status = code.unwrap_or(0) == 0;
                    } else if !state.status.is_terminal() {
                        state.status = if code.unwrap_or(1) == 0 {
                            should_fetch_status = true;
                            ClaudeAuthLoginStatus::Succeeded
                        } else {
                            ClaudeAuthLoginStatus::Failed
                        };
                        if state.status == ClaudeAuthLoginStatus::Failed && state.error.is_none() {
                            state.error = Some(format!(
                                "Claude Code auth exited with code {}.",
                                code.map(|value| value.to_string())
                                    .unwrap_or_else(|| "unknown".to_owned())
                            ));
                        }
                    }
                })
                .await;
            if should_fetch_status {
                let auth_status = fetch_auth_status(&command_spec).await;
                session
                    .update(|state| match auth_status {
                        Ok(value) => {
                            state.status = ClaudeAuthLoginStatus::Succeeded;
                            state.auth_status = Some(value);
                            state.error = None;
                        }
                        Err(error) => {
                            state.status = ClaudeAuthLoginStatus::Succeeded;
                            state.error = Some(error);
                        }
                    })
                    .await;
            }
        }
        Err(error) => {
            session
                .update(|state| {
                    state.status = ClaudeAuthLoginStatus::Failed;
                    state.error = Some(format!("Failed waiting for Claude Code auth: {error}"));
                })
                .await;
        }
    }

    let final_snapshot = session.snapshot().await;
    if final_snapshot.status == ClaudeAuthLoginStatus::Failed {
        send_start_error_once(
            &mut started_tx,
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "claude_auth_failed_before_url",
                final_snapshot.error.clone().unwrap_or_else(|| {
                    "Claude Code auth failed before returning a login URL.".to_owned()
                }),
            ),
        );
    }
}

async fn handle_auth_event(
    session: &Arc<ClaudeAuthSession>,
    command_spec: &AuthCommandSpec,
    event: &Value,
    started_tx: &mut Option<oneshot::Sender<Result<ClaudeAuthLoginResponse, ApiError>>>,
) {
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match event_type {
        "authorization_url" => {
            let url = event
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned();
            if url.is_empty() {
                session
                    .update(|state| {
                        state.status = ClaudeAuthLoginStatus::Failed;
                        state.error =
                            Some("Claude Code auth emitted an empty authorization URL.".to_owned());
                    })
                    .await;
                send_start_error_once(
                    started_tx,
                    ApiError::new(
                        StatusCode::BAD_GATEWAY,
                        "empty_authorization_url",
                        "Claude Code auth emitted an empty authorization URL.",
                    ),
                );
                return;
            }
            session
                .update(|state| {
                    state.status = ClaudeAuthLoginStatus::WaitingForCode;
                    state.url = Some(url);
                    state.error = None;
                })
                .await;
            send_start_response_once(started_tx, session.snapshot().await);
        }
        "input_requested" => {
            session
                .update(|state| {
                    if !state.status.is_terminal() {
                        state.status = ClaudeAuthLoginStatus::WaitingForCode;
                    }
                })
                .await;
        }
        "success" => {
            let auth_status = fetch_auth_status(command_spec).await;
            session
                .update(|state| {
                    state.status = ClaudeAuthLoginStatus::Succeeded;
                    match auth_status {
                        Ok(value) => {
                            state.auth_status = Some(value);
                            state.error = None;
                        }
                        Err(error) => {
                            state.error = Some(error);
                        }
                    }
                })
                .await;
        }
        "error" => {
            let message = event
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| event.get("error").and_then(Value::as_str))
                .unwrap_or("Claude Code auth failed.")
                .to_owned();
            session
                .update(|state| {
                    state.status = ClaudeAuthLoginStatus::Failed;
                    state.error = Some(message.clone());
                })
                .await;
            send_start_error_once(
                started_tx,
                ApiError::new(StatusCode::BAD_GATEWAY, "claude_auth_failed", message),
            );
        }
        "exit" => {
            let code = event
                .get("exit_code")
                .and_then(Value::as_i64)
                .and_then(|value| i32::try_from(value).ok());
            session
                .update(|state| {
                    state.exit_code = code;
                    if code.unwrap_or(1) != 0 && !state.status.is_terminal() {
                        state.status = ClaudeAuthLoginStatus::Failed;
                        state.error = Some(format!(
                            "Claude Code auth exited with code {}.",
                            code.map(|value| value.to_string())
                                .unwrap_or_else(|| "unknown".to_owned())
                        ));
                    }
                })
                .await;
        }
        _ => {}
    }
}

async fn fetch_auth_status(command_spec: &AuthCommandSpec) -> Result<Value, String> {
    let output = Command::new(&command_spec.program)
        .args(&command_spec.prefix_args)
        .args(["auth", "status", "--json"])
        .output()
        .await
        .map_err(|error| format!("Failed to run Claude Code auth status: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(if detail.is_empty() {
            format!("Claude Code auth status exited with {}.", output.status)
        } else {
            detail
        });
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Claude Code auth status returned invalid JSON: {error}"))
}

async fn drain_stderr(login_id: String, stderr: tokio::process::ChildStderr) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        debug!(login_id = %login_id, line = %line, "Claude auth stderr");
    }
}

fn build_login_args(request: &StartClaudeAuthRequest) -> Vec<String> {
    let mut args = vec![
        "auth".to_owned(),
        "login".to_owned(),
        "--json-events".to_owned(),
    ];
    match request.mode {
        ClaudeAuthLoginMode::Claudeai => args.push("--claudeai".to_owned()),
        ClaudeAuthLoginMode::Console => args.push("--console".to_owned()),
    }
    if request.sso {
        args.push("--sso".to_owned());
    }
    if let Some(email) = request
        .email
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push("--email".to_owned());
        args.push(email.to_owned());
    }
    args
}

async fn resolve_auth_command(state: &Arc<AppState>) -> Result<AuthCommandSpec, ApiError> {
    if let Some(spec) = state.ops.provider_auth_sessions.command_override().await {
        return Ok(spec);
    }
    let config = state.config_snapshot();
    let Some(spec) = resolve_auth_command_from_config(&config) else {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "claude_auth_command_unavailable",
            "Claude Code auth command was not found. Configure GARYX_CCTTY_PATH or install cctty next to garyx or on PATH.",
        ));
    };
    Ok(spec)
}

fn resolve_auth_command_from_config(config: &GaryxConfig) -> Option<AuthCommandSpec> {
    let agent_cfg = config
        .agents
        .get("claude")
        .and_then(|value| serde_json::from_value::<AgentProviderConfig>(value.clone()).ok());
    let agent_cfg = agent_cfg.as_ref();
    if let Some(path) = explicit_cctty_path(agent_cfg) {
        return Some(AuthCommandSpec {
            program: path,
            prefix_args: Vec::new(),
        });
    }
    bundled_cctty_path()
        .or_else(|| executable_on_path(CCTTY_BINARY_NAME))
        .map(|program| AuthCommandSpec {
            program,
            prefix_args: Vec::new(),
        })
        .or_else(|| {
            std::env::current_exe().ok().map(|program| AuthCommandSpec {
                program,
                prefix_args: vec![EMBEDDED_CCTTY_ARG.to_owned()],
            })
        })
}

fn explicit_cctty_path(agent_cfg: Option<&AgentProviderConfig>) -> Option<PathBuf> {
    explicit_env_path(agent_cfg, GARYX_CCTTY_PATH_ENV)
        .or_else(|| std::env::var_os(GARYX_CCTTY_PATH_ENV).and_then(nonempty_os_path))
        .or_else(|| {
            (claude_cli_mode(agent_cfg) != "native")
                .then(|| {
                    agent_cfg
                        .and_then(|cfg| {
                            let path = cfg.claude_cli_path.trim();
                            (!path.is_empty()).then(|| PathBuf::from(path))
                        })
                        .or_else(|| explicit_env_path(agent_cfg, GARYX_CLAUDE_CLI_PATH_ENV))
                        .or_else(|| {
                            std::env::var_os(GARYX_CLAUDE_CLI_PATH_ENV).and_then(nonempty_os_path)
                        })
                })
                .flatten()
        })
}

fn explicit_env_path(agent_cfg: Option<&AgentProviderConfig>, key: &str) -> Option<PathBuf> {
    agent_cfg.and_then(|cfg| {
        cfg.env
            .get(key)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    })
}

fn nonempty_os_path(value: std::ffi::OsString) -> Option<PathBuf> {
    let path = PathBuf::from(value);
    (!path.as_os_str().is_empty()).then_some(path)
}

fn claude_cli_mode(agent_cfg: Option<&AgentProviderConfig>) -> String {
    let raw = agent_cfg
        .and_then(|cfg| cfg.env.get(GARYX_CLAUDE_CLI_MODE_ENV).cloned())
        .or_else(|| std::env::var(GARYX_CLAUDE_CLI_MODE_ENV).ok())
        .or_else(|| agent_cfg.map(|cfg| cfg.claude_cli_mode.clone()))
        .unwrap_or_else(garyx_models::provider::default_claude_cli_mode);
    let raw = raw.trim();
    if raw.is_empty() {
        garyx_models::provider::default_claude_cli_mode()
    } else {
        raw.to_ascii_lowercase()
    }
}

fn bundled_cctty_path() -> Option<PathBuf> {
    let current_exe = std::env::current_exe().ok()?;
    let dir = current_exe.parent()?;
    let candidate = dir.join(CCTTY_BINARY_NAME);
    executable_file_exists(&candidate).then_some(candidate)
}

fn executable_on_path(name: &str) -> Option<PathBuf> {
    let path_env = std::env::var_os("PATH")?;
    std::env::split_paths(&path_env)
        .map(|dir| dir.join(name))
        .find(|candidate| executable_file_exists(candidate))
}

fn executable_file_exists(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn send_start_response_once(
    started_tx: &mut Option<oneshot::Sender<Result<ClaudeAuthLoginResponse, ApiError>>>,
    response: ClaudeAuthLoginResponse,
) {
    if let Some(tx) = started_tx.take() {
        let _ = tx.send(Ok(response));
    }
}

fn send_start_error_once(
    started_tx: &mut Option<oneshot::Sender<Result<ClaudeAuthLoginResponse, ApiError>>>,
    error: ApiError,
) {
    if let Some(tx) = started_tx.take() {
        let _ = tx.send(Err(error));
    }
}

fn unknown_session_error(login_id: &str) -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "unknown_claude_auth_session",
        format!("Claude Code auth session '{login_id}' was not found."),
    )
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    fn io(status: StatusCode, code: &'static str, error: std::io::Error) -> Self {
        warn!(code, error = %error, "Claude Code auth API IO error");
        Self::new(status, code, error.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": {
                    "code": self.code,
                    "message": self.message,
                }
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::StatusCode;
    use garyx_models::config::GaryxConfig;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;
    use tower::ServiceExt;

    #[tokio::test]
    async fn claude_auth_api_starts_submits_and_reports_status() {
        let dir = tempdir().unwrap();
        let fake_cctty = write_fake_cctty(dir.path());
        let config = crate::test_support::with_gateway_auth(GaryxConfig::default());
        let state = crate::server::AppStateBuilder::new(config).build();
        state
            .ops
            .provider_auth_sessions
            .set_command_override_for_test(fake_cctty)
            .await;
        let router = crate::route_graph::build_router(state);

        let start_response = router
            .clone()
            .oneshot(
                crate::test_support::authed_request()
                    .method("POST")
                    .uri("/api/providers/claude_code/auth/start")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"mode":"claudeai"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(start_response.status(), StatusCode::CREATED);
        let start: ClaudeAuthLoginResponse = serde_json::from_slice(
            &to_bytes(start_response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(start.status, ClaudeAuthLoginStatus::WaitingForCode);
        assert_eq!(
            start.url.as_deref(),
            Some("https://claude.ai/oauth/authorize?state=test")
        );
        assert!(!start.login_id.is_empty());

        let submit_uri = format!("/api/providers/claude_code/auth/{}/submit", start.login_id);
        let submit_response = router
            .clone()
            .oneshot(
                crate::test_support::authed_request()
                    .method("POST")
                    .uri(submit_uri)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"token":"TEST-CODE"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(submit_response.status(), StatusCode::OK);
        let submitted: ClaudeAuthLoginResponse = serde_json::from_slice(
            &to_bytes(submit_response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(submitted.status, ClaudeAuthLoginStatus::Submitted);

        let status_uri = format!("/api/providers/claude_code/auth/{}", start.login_id);
        let mut final_snapshot = None;
        for _ in 0..30 {
            let status_response = router
                .clone()
                .oneshot(
                    crate::test_support::authed_request()
                        .method("GET")
                        .uri(&status_uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(status_response.status(), StatusCode::OK);
            let snapshot: ClaudeAuthLoginResponse = serde_json::from_slice(
                &to_bytes(status_response.into_body(), usize::MAX)
                    .await
                    .unwrap(),
            )
            .unwrap();
            if snapshot.status == ClaudeAuthLoginStatus::Succeeded {
                final_snapshot = Some(snapshot);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let final_snapshot = final_snapshot.expect("auth session should succeed");
        assert_eq!(final_snapshot.exit_code, Some(0));
        assert_eq!(
            final_snapshot
                .auth_status
                .as_ref()
                .and_then(|value| value.get("authMethod"))
                .and_then(Value::as_str),
            Some("claude.ai")
        );
        assert_eq!(
            final_snapshot
                .auth_status
                .as_ref()
                .and_then(|value| value.get("orgName"))
                .and_then(Value::as_str),
            Some("Test Org")
        );
    }

    #[tokio::test]
    async fn claude_auth_submit_rejects_unknown_login_id() {
        let config = crate::test_support::with_gateway_auth(GaryxConfig::default());
        let state = crate::server::AppStateBuilder::new(config).build();
        let router = crate::route_graph::build_router(state);
        let response = router
            .oneshot(
                crate::test_support::authed_request()
                    .method("POST")
                    .uri("/api/providers/claude_code/auth/missing/submit")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"code":"TEST-CODE"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn auth_command_does_not_fall_back_to_native_claude() {
        let mut config = GaryxConfig::default();
        config.agents.insert(
            "claude".to_owned(),
            serde_json::to_value(AgentProviderConfig {
                provider_id: "claude".to_owned(),
                provider_type: garyx_models::provider::ProviderType::ClaudeCode
                    .as_slug()
                    .to_owned(),
                claude_cli_mode: "native".to_owned(),
                claude_cli_path: String::new(),
                ..AgentProviderConfig::default()
            })
            .unwrap(),
        );

        let command = resolve_auth_command_from_config(&config).unwrap();
        assert_ne!(
            command.program.file_name().and_then(|value| value.to_str()),
            Some("claude")
        );
    }

    #[test]
    fn auth_command_accepts_cctty_path_override_in_native_mode() {
        let mut config = GaryxConfig::default();
        let mut provider = AgentProviderConfig {
            provider_id: "claude".to_owned(),
            provider_type: garyx_models::provider::ProviderType::ClaudeCode
                .as_slug()
                .to_owned(),
            claude_cli_mode: "native".to_owned(),
            claude_cli_path: "/opt/test/claude".to_owned(),
            ..AgentProviderConfig::default()
        };
        provider.env.insert(
            GARYX_CCTTY_PATH_ENV.to_owned(),
            "/opt/test/cctty".to_owned(),
        );
        config
            .agents
            .insert("claude".to_owned(), serde_json::to_value(provider).unwrap());

        let command = resolve_auth_command_from_config(&config).unwrap();
        assert_eq!(command.program, PathBuf::from("/opt/test/cctty"));
        assert!(command.prefix_args.is_empty());
    }

    fn write_fake_cctty(dir: &Path) -> PathBuf {
        let path = dir.join("fake-cctty.py");
        fs::write(
            &path,
            r#"#!/usr/bin/env python3
import json
import sys

args = sys.argv[1:]
if args == ["auth", "status", "--json"]:
    print(json.dumps({
        "authMethod": "claude.ai",
        "orgName": "Test Org",
        "subscriptionType": "team"
    }), flush=True)
    sys.exit(0)

if args[:3] == ["auth", "login", "--json-events"]:
    print(json.dumps({"type": "started"}), flush=True)
    print(json.dumps({"type": "authorization_url", "url": "https://claude.ai/oauth/authorize?state=test"}), flush=True)
    print(json.dumps({"type": "input_requested", "input": "authorization_code"}), flush=True)
    code = sys.stdin.readline().strip()
    if code == "TEST-CODE":
        print(json.dumps({"type": "success", "message": "Login successful."}), flush=True)
        print(json.dumps({"type": "exit", "exit_code": 0}), flush=True)
        sys.exit(0)
    print(json.dumps({"type": "error", "message": "bad code"}), flush=True)
    print(json.dumps({"type": "exit", "exit_code": 1}), flush=True)
    sys.exit(1)

print("unexpected args: " + repr(args), file=sys.stderr)
sys.exit(2)
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).unwrap();
        }
        path
    }
}
