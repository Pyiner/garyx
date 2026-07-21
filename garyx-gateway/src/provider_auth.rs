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
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::provider_accounts::{self, AccountsApiError, ClaudeAuthTarget};
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

    async fn remove(&self, login_id: &str) -> Option<Arc<ClaudeAuthSession>> {
        self.sessions.lock().await.remove(login_id)
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
    target: ClaudeAuthTarget,
    state: Mutex<ClaudeAuthSessionState>,
    input: Mutex<Option<AuthLoginInput>>,
    cancellation: CancellationToken,
}

impl ClaudeAuthSession {
    fn new(login_id: String, input: AuthLoginInput, target: ClaudeAuthTarget) -> Self {
        Self {
            login_id,
            target,
            state: Mutex::new(ClaudeAuthSessionState {
                status: ClaudeAuthLoginStatus::Starting,
                url: None,
                auth_status: None,
                error: None,
                exit_code: None,
            }),
            input: Mutex::new(Some(input)),
            cancellation: CancellationToken::new(),
        }
    }

    fn login_id(&self) -> String {
        self.login_id.clone()
    }

    async fn snapshot(&self) -> ClaudeAuthLoginResponse {
        self.state
            .lock()
            .await
            .to_response(&self.login_id, self.target.account_id.clone())
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

    async fn cancel(&self) -> ClaudeAuthLoginResponse {
        self.update(|state| {
            if !state.status.is_terminal() {
                state.status = ClaudeAuthLoginStatus::Failed;
                state.error = Some("Claude Code sign-in was cancelled.".to_owned());
            }
        })
        .await;
        self.close_input().await;
        self.cancellation.cancel();
        self.snapshot().await
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
    fn to_response(&self, login_id: &str, account_id: Option<String>) -> ClaudeAuthLoginResponse {
        ClaudeAuthLoginResponse {
            login_id: login_id.to_owned(),
            account_id,
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
    /// Present only when desktop is creating a new isolated profile.
    managed_account_name: Option<String>,
    /// Present only when desktop is reauthenticating an existing profile.
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SubmitClaudeAuthRequest {
    code: Option<String>,
    token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaudeAuthLoginResponse {
    pub login_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
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
    let target = provider_accounts::prepare_auth_target(
        &state,
        request.managed_account_name.as_deref(),
        request.account_id.as_deref(),
    )
    .await
    .map_err(map_accounts_error)?;
    let login_args = build_login_args(&request);
    let claude_path = state
        .ops
        .provider_auth_sessions
        .claude_path_override()
        .await;
    let auth_options = AuthLoginOptions {
        passthrough_args: login_args,
        claude_path: claude_path.clone(),
        env: target.environment(),
        ..AuthLoginOptions::default()
    };
    let mut auth_session = match AuthLoginSession::start(auth_options) {
        Ok(session) => session,
        Err(error) => {
            provider_accounts::cleanup_failed_auth_target(&state, &target).await;
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "spawn_claude_auth_failed",
                error.to_string(),
            ));
        }
    };
    let input = auth_session.input();
    let events = auth_session.take_events();
    let session = Arc::new(ClaudeAuthSession::new(login_id.clone(), input, target));
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
        state,
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
            session.cancel().await;
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

pub async fn cancel_claude_code_auth(
    State(state): State<Arc<AppState>>,
    AxumPath(login_id): AxumPath<String>,
) -> Result<Json<ClaudeAuthLoginResponse>, ApiError> {
    let session = state
        .ops
        .provider_auth_sessions
        .remove(&login_id)
        .await
        .ok_or_else(|| unknown_session_error(&login_id))?;
    Ok(Json(session.cancel().await))
}

async fn read_auth_events(
    session: Arc<ClaudeAuthSession>,
    mut events: tokio::sync::mpsc::Receiver<AuthLoginEvent>,
    auth_session: AuthLoginSession,
    app_state: Arc<AppState>,
    claude_path: Option<PathBuf>,
    started_tx: oneshot::Sender<Result<ClaudeAuthLoginResponse, ApiError>>,
) {
    let mut started_tx = Some(started_tx);
    loop {
        tokio::select! {
            _ = session.cancellation.cancelled() => {
                session.close_input().await;
                if let Err(error) = auth_session.shutdown().await {
                    tracing::warn!(
                        login_id = %session.login_id,
                        error = %error,
                        "failed to shut down Claude Code auth process"
                    );
                }
                drop(events);
                provider_accounts::cleanup_failed_auth_target(&app_state, &session.target).await;
                return;
            }
            event = events.recv() => {
                let Some(event) = event else {
                    break;
                };
                handle_auth_event(&session, event, &mut started_tx).await;
            }
        }
    }

    session.close_input().await;
    match auth_session.wait().await {
        Ok(code) => {
            let code = Some(code);
            let mut should_finalize = false;
            session
                .update(|state| {
                    state.exit_code = code;
                    if !state.status.is_terminal() {
                        state.status = if code.unwrap_or(1) == 0 {
                            should_finalize = true;
                            ClaudeAuthLoginStatus::Submitted
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
            if should_finalize {
                let auth_status =
                    fetch_auth_status(claude_path, session.target.environment()).await;
                let (value, warning) = match auth_status {
                    Ok(value) => (value, None),
                    Err(error) => (json!({}), Some(error)),
                };
                let completion =
                    provider_accounts::complete_auth_target(&app_state, &session.target, &value)
                        .await;
                session
                    .update(|state| match completion {
                        Ok(()) => {
                            state.status = ClaudeAuthLoginStatus::Succeeded;
                            state.auth_status = Some(value);
                            state.error = warning;
                        }
                        Err(error) => {
                            state.status = ClaudeAuthLoginStatus::Failed;
                            state.error = Some(error.to_string());
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
        provider_accounts::cleanup_failed_auth_target(&app_state, &session.target).await;
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
            session
                .update(|state| {
                    if !state.status.is_terminal() {
                        // The process can still fail after printing the success
                        // line. Finalize credentials/config only after exit 0.
                        state.status = ClaudeAuthLoginStatus::Submitted;
                        state.error = None;
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

async fn fetch_auth_status(
    claude_path: Option<PathBuf>,
    env: HashMap<String, String>,
) -> Result<Value, String> {
    auth_status_json(AuthStatusOptions {
        claude_path,
        env,
        ..AuthStatusOptions::default()
    })
    .await
    .map_err(|error| format!("Failed to run Claude Code auth status: {error}"))
}

fn map_accounts_error(error: AccountsApiError) -> ApiError {
    let (status, code, message) = error.into_parts();
    ApiError::new(status, code, message)
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
    use std::time::Instant;
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
    async fn cancelling_managed_auth_closes_process_and_removes_uncommitted_profile() {
        let dir = tempdir().unwrap();
        let fake_claude = write_fake_claude(dir.path());
        let auth_pid_path = dir.path().join("auth-login.pid");
        let config = crate::test_support::with_gateway_auth(GaryxConfig::default());
        let state = crate::server::AppStateBuilder::new(config)
            .with_config_path(dir.path().join("config.yaml"))
            .build();
        state
            .ops
            .provider_auth_sessions
            .set_claude_path_override_for_test(fake_claude)
            .await;
        let router = crate::route_graph::build_router(state.clone());

        let start_response = router
            .clone()
            .oneshot(
                crate::test_support::authed_request()
                    .method("POST")
                    .uri("/api/providers/claude_code/auth/start")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"mode":"claudeai","managed_account_name":"Cancelled"}"#,
                    ))
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
        let account_id = start.account_id.as_deref().expect("managed account id");
        let auth_pid = fs::read_to_string(&auth_pid_path)
            .unwrap()
            .trim()
            .parse::<libc::pid_t>()
            .unwrap();
        let account_dir = dir
            .path()
            .join("provider-accounts/claude-code")
            .join(account_id);
        assert!(account_dir.is_dir());

        let auth_uri = format!("/api/providers/claude_code/auth/{}", start.login_id);
        let cancel_response = router
            .clone()
            .oneshot(
                crate::test_support::authed_request()
                    .method("DELETE")
                    .uri(&auth_uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cancel_response.status(), StatusCode::OK);
        let cancelled: ClaudeAuthLoginResponse = serde_json::from_slice(
            &to_bytes(cancel_response.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(cancelled.status, ClaudeAuthLoginStatus::Failed);
        assert_eq!(
            cancelled.error.as_deref(),
            Some("Claude Code sign-in was cancelled.")
        );

        for _ in 0..100 {
            if !account_dir.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!account_dir.exists(), "cancelled profile should be removed");
        assert!(
            state
                .config_snapshot()
                .provider_accounts
                .claude_code
                .accounts
                .is_empty()
        );
        assert_child_reaped(auth_pid).await;

        let status_response = router
            .oneshot(
                crate::test_support::authed_request()
                    .method("GET")
                    .uri(auth_uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(status_response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn managed_auth_uses_isolated_config_dir_and_commits_account() {
        let dir = tempdir().unwrap();
        let fake_claude = write_fake_claude(dir.path());
        let mut config = crate::test_support::with_gateway_auth(GaryxConfig::default());
        config.agents.insert(
            "claude".to_owned(),
            serde_json::to_value(AgentProviderConfig {
                provider_id: "claude".to_owned(),
                provider_type: garyx_models::ProviderType::ClaudeCode.as_slug().to_owned(),
                claude_cli_mode: "native".to_owned(),
                claude_cli_path: fake_claude.to_string_lossy().into_owned(),
                ..AgentProviderConfig::default()
            })
            .unwrap(),
        );
        let state = crate::server::AppStateBuilder::new(config)
            .with_config_path(dir.path().join("config.yaml"))
            .build();
        state
            .ops
            .provider_auth_sessions
            .set_claude_path_override_for_test(fake_claude)
            .await;
        let router = crate::route_graph::build_router(state.clone());

        let start_response = router
            .clone()
            .oneshot(
                crate::test_support::authed_request()
                    .method("POST")
                    .uri("/api/providers/claude_code/auth/start")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"mode":"claudeai","managed_account_name":"Work"}"#,
                    ))
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
        let account_id = start.account_id.clone().expect("managed account id");

        let submit_response = router
            .clone()
            .oneshot(
                crate::test_support::authed_request()
                    .method("POST")
                    .uri(format!(
                        "/api/providers/claude_code/auth/{}/submit",
                        start.login_id
                    ))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"code":"TEST-CODE"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(submit_response.status(), StatusCode::OK);

        let status_uri = format!("/api/providers/claude_code/auth/{}", start.login_id);
        let mut final_snapshot = None;
        let mut last_snapshot = None;
        for _ in 0..500 {
            let response = router
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
            let snapshot: ClaudeAuthLoginResponse =
                serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                    .unwrap();
            if snapshot.status == ClaudeAuthLoginStatus::Succeeded {
                final_snapshot = Some(snapshot);
                break;
            }
            if snapshot.status == ClaudeAuthLoginStatus::Failed {
                final_snapshot = Some(snapshot);
                break;
            }
            last_snapshot = Some(snapshot);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let final_snapshot = final_snapshot.unwrap_or_else(|| {
            panic!("managed auth should finish; last snapshot: {last_snapshot:?}")
        });
        assert_eq!(
            final_snapshot.status,
            ClaudeAuthLoginStatus::Succeeded,
            "managed auth failed: {:?}",
            final_snapshot.error
        );

        let config = state.config_snapshot();
        // Adding an account must not change the active selection: the
        // pre-existing selection (System default here) stays in place.
        assert_eq!(
            config
                .provider_accounts
                .claude_code
                .active_account_id
                .as_deref(),
            None
        );
        let account = config
            .provider_accounts
            .claude_code
            .account(&account_id)
            .expect("committed account");
        assert_eq!(account.name, "Work");
        assert_eq!(account.organization.as_deref(), Some("Test Org"));
        let account_dir = dir
            .path()
            .join("provider-accounts/claude-code")
            .join(account_id);
        assert!(account_dir.join("login-env-seen").is_file());
        assert!(account_dir.join("status-env-seen").is_file());
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
import os
import sys
from pathlib import Path

args = sys.argv[1:]
config_dir = os.environ.get("CLAUDE_CONFIG_DIR")
if args == ["auth", "status", "--json"]:
    if config_dir:
        Path(config_dir, "status-env-seen").write_text(config_dir, encoding="utf-8")
    print(json.dumps({
        "authMethod": "claude.ai",
        "orgName": "Test Org",
        "subscriptionType": "team"
    }), flush=True)
    sys.exit(0)

if args[:2] == ["auth", "login"]:
    Path(__file__).with_name("auth-login.pid").write_text(str(os.getpid()), encoding="utf-8")
    if config_dir:
        Path(config_dir, "login-env-seen").write_text(config_dir, encoding="utf-8")
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

    async fn assert_child_reaped(pid: libc::pid_t) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let result = unsafe { libc::kill(pid, 0) };
            if result != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                return;
            }
            if Instant::now() >= deadline {
                let mut status = 0;
                let wait_result = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
                if wait_result == 0 {
                    unsafe {
                        libc::kill(-pid, libc::SIGKILL);
                        libc::kill(pid, libc::SIGKILL);
                        libc::waitpid(pid, &mut status, 0);
                    }
                }
                panic!("auth login child {pid} was not reaped; waitpid returned {wait_result}");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
