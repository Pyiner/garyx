use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use cctty::CcttyError;
use cctty::auth::{
    AuthLoginEvent, AuthLoginInput, AuthLoginOptions, AuthLoginSession, AuthStatusOptions,
    auth_status_json,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, oneshot};
use uuid::Uuid;

use crate::server::AppState;

const AUTH_START_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct ClaudeAuthSessionStore {
    sessions: Mutex<HashMap<String, Arc<ClaudeAuthSession>>>,
    #[cfg(test)]
    claude_path_override: Mutex<Option<PathBuf>>,
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

    #[cfg(test)]
    async fn claude_path_override(&self) -> Option<PathBuf> {
        self.claude_path_override.lock().await.clone()
    }

    #[cfg(not(test))]
    async fn claude_path_override(&self) -> Option<PathBuf> {
        None
    }

    #[cfg(test)]
    pub async fn set_claude_path_override_for_test(&self, command: PathBuf) {
        *self.claude_path_override.lock().await = Some(command);
    }
}

struct ClaudeAuthSession {
    login_id: String,
    state: Mutex<ClaudeAuthSessionState>,
    input: Mutex<Option<AuthLoginInput>>,
}

impl ClaudeAuthSession {
    fn new(login_id: String, input: AuthLoginInput) -> Self {
        Self {
            login_id,
            state: Mutex::new(ClaudeAuthSessionState {
                status: ClaudeAuthLoginStatus::Starting,
                url: None,
                auth_status: None,
                error: None,
                exit_code: None,
            }),
            input: Mutex::new(Some(input)),
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
        let input = self.input.lock().await.clone();
        let Some(input) = input else {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "claude_auth_session_closed",
                "Claude Code auth session is no longer accepting input.",
            ));
        };
        input
            .submit_code(code.to_owned())
            .await
            .map_err(map_submit_code_error)?;

        self.update(|state| {
            if !state.status.is_terminal() {
                state.status = ClaudeAuthLoginStatus::Submitted;
                state.error = None;
            }
        })
        .await;
        Ok(self.snapshot().await)
    }

    async fn close_input(&self) {
        if let Some(input) = self.input.lock().await.take() {
            input.close();
        }
    }
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
    let login_id = Uuid::new_v4().to_string();
    let login_args = build_login_args(&request);
    let claude_path = state
        .ops
        .provider_auth_sessions
        .claude_path_override()
        .await;
    let mut auth_session = AuthLoginSession::start(AuthLoginOptions {
        passthrough_args: login_args,
        claude_path: claude_path.clone(),
        ..AuthLoginOptions::default()
    })
    .map_err(|error| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "spawn_claude_auth_failed",
            error.to_string(),
        )
    })?;
    let input = auth_session.input();
    let events = auth_session.take_events();
    let session = Arc::new(ClaudeAuthSession::new(login_id.clone(), input));
    state
        .ops
        .provider_auth_sessions
        .insert(session.clone())
        .await;
    let (started_tx, started_rx) = oneshot::channel();
    tokio::spawn(read_auth_events(
        session.clone(),
        events,
        auth_session,
        claude_path,
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
    mut events: tokio::sync::mpsc::Receiver<AuthLoginEvent>,
    auth_session: AuthLoginSession,
    claude_path: Option<PathBuf>,
    started_tx: oneshot::Sender<Result<ClaudeAuthLoginResponse, ApiError>>,
) {
    let mut started_tx = Some(started_tx);
    while let Some(event) = events.recv().await {
        handle_auth_event(&session, claude_path.clone(), event, &mut started_tx).await;
    }

    session.close_input().await;
    match auth_session.wait().await {
        Ok(code) => {
            let code = Some(code);
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
                let auth_status = fetch_auth_status(claude_path).await;
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
    claude_path: Option<PathBuf>,
    event: AuthLoginEvent,
    started_tx: &mut Option<oneshot::Sender<Result<ClaudeAuthLoginResponse, ApiError>>>,
) {
    match event {
        AuthLoginEvent::AuthorizationUrl { url } => {
            let url = url.trim().to_owned();
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
        AuthLoginEvent::InputRequested { .. } => {
            session
                .update(|state| {
                    if !state.status.is_terminal() {
                        state.status = ClaudeAuthLoginStatus::WaitingForCode;
                    }
                })
                .await;
        }
        AuthLoginEvent::Success { .. } => {
            let auth_status = fetch_auth_status(claude_path).await;
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
        AuthLoginEvent::Error { message } => {
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
        AuthLoginEvent::Exit { exit_code } => {
            let code = Some(exit_code);
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
        AuthLoginEvent::Started { .. } => {}
    }
}

async fn fetch_auth_status(claude_path: Option<PathBuf>) -> Result<Value, String> {
    auth_status_json(AuthStatusOptions {
        claude_path,
        ..AuthStatusOptions::default()
    })
    .await
    .map_err(|error| format!("Failed to run Claude Code auth status: {error}"))
}

fn map_submit_code_error(error: CcttyError) -> ApiError {
    if matches!(&error, CcttyError::Tty(message) if message.contains("session is closed")) {
        return ApiError::new(
            StatusCode::CONFLICT,
            "claude_auth_session_closed",
            "Claude Code auth session is no longer accepting input.",
        );
    }
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        "write_auth_code_failed",
        error.to_string(),
    )
}

fn build_login_args(request: &StartClaudeAuthRequest) -> Vec<String> {
    let mut args = vec!["auth".to_owned(), "login".to_owned()];
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
    use garyx_models::config::{AgentProviderConfig, GaryxConfig};
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;
    use tower::ServiceExt;

    #[tokio::test]
    async fn claude_auth_api_starts_submits_and_reports_status() {
        let dir = tempdir().unwrap();
        let fake_claude = write_fake_claude(dir.path());
        let config = crate::test_support::with_gateway_auth(GaryxConfig::default());
        let state = crate::server::AppStateBuilder::new(config).build();
        state
            .ops
            .provider_auth_sessions
            .set_claude_path_override_for_test(fake_claude)
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

    #[tokio::test]
    async fn claude_auth_ignores_cctty_config_and_native_mode() {
        let dir = tempdir().unwrap();
        let fake_claude = write_fake_claude(dir.path());
        let failing_cctty = write_failing_executable(dir.path().join("old-cctty"));
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
            "GARYX_CCTTY_PATH".to_owned(),
            failing_cctty.to_string_lossy().to_string(),
        );
        config
            .agents
            .insert("claude".to_owned(), serde_json::to_value(provider).unwrap());
        let config = crate::test_support::with_gateway_auth(config);
        let state = crate::server::AppStateBuilder::new(config).build();
        state
            .ops
            .provider_auth_sessions
            .set_claude_path_override_for_test(fake_claude)
            .await;
        let router = crate::route_graph::build_router(state);

        let start_response = router
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
    }

    fn write_fake_claude(dir: &Path) -> PathBuf {
        let path = dir.join("claude");
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

if args[:2] == ["auth", "login"]:
    print("Opening browser to sign in...", flush=True)
    print("If the browser did not open, visit: https://claude.ai/oauth/authorize?state=test", flush=True)
    sys.stdout.write("Paste code here if prompted > ")
    sys.stdout.flush()
    code = sys.stdin.readline().strip()
    if code == "TEST-CODE":
        print("Login successful.", flush=True)
        sys.exit(0)
    print("Login failed.", flush=True)
    sys.exit(1)

print("unexpected args: " + repr(args), file=sys.stderr)
sys.exit(2)
"#,
        )
        .unwrap();
        make_executable(&path);
        path
    }

    fn write_failing_executable(path: PathBuf) -> PathBuf {
        fs::write(
            &path,
            "#!/bin/sh\necho old cctty should not be launched >&2\nexit 99\n",
        )
        .unwrap();
        make_executable(&path);
        path
    }

    fn make_executable(path: &Path) {
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).unwrap();
        }
    }
}
