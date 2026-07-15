use std::time::Duration;

use chrono::{DateTime, Utc};
use garyx_models::{
    CodingUsageSnapshot, CustomAgentProfile, QuotaCheckError, QuotaCredentialScope, QuotaScope,
    QuotaStatus, evaluate_quota,
};
use serde::de::DeserializeOwned;

use super::{GatewayEndpoint, format_local_timestamp, gateway_request};

pub(super) const TASK_CREATE_QUOTA_CHECK_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) async fn check_agent_quota(
    gateway: &GatewayEndpoint,
    agent_id: &str,
) -> Result<QuotaStatus, QuotaCheckError> {
    check_agent_quota_with_options(
        gateway,
        agent_id,
        Utc::now(),
        TASK_CREATE_QUOTA_CHECK_TIMEOUT,
    )
    .await
}

pub(super) async fn check_agent_quota_with_options(
    gateway: &GatewayEndpoint,
    agent_id: &str,
    now: DateTime<Utc>,
    timeout: Duration,
) -> Result<QuotaStatus, QuotaCheckError> {
    match tokio::time::timeout(
        timeout,
        check_agent_quota_inner(gateway, agent_id, now, timeout),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(QuotaCheckError::TimedOut),
    }
}

async fn check_agent_quota_inner(
    gateway: &GatewayEndpoint,
    agent_id: &str,
    now: DateTime<Utc>,
    request_timeout: Duration,
) -> Result<QuotaStatus, QuotaCheckError> {
    let client = reqwest::Client::new();
    let profile_path = format!("/api/custom-agents/{}", urlencoding::encode(agent_id));
    let profile: CustomAgentProfile = fetch_gateway_json_once(
        &client,
        gateway,
        &profile_path,
        request_timeout,
        "agent profile",
    )
    .await?;

    let credential_scope = if profile.provider_env.is_empty() {
        QuotaCredentialScope::DefaultLocal
    } else {
        // Do not copy keys or values into an error: any override is enough to
        // prove that the gateway's default-local usage reading may not align.
        QuotaCredentialScope::Customized
    };
    if credential_scope == QuotaCredentialScope::Customized {
        return Err(QuotaCheckError::CredentialScopeMismatch);
    }

    let snapshot: CodingUsageSnapshot = fetch_gateway_json_once(
        &client,
        gateway,
        "/api/usage/coding",
        request_timeout,
        "coding usage",
    )
    .await?;
    let model = profile.model.trim();
    evaluate_quota(
        &snapshot,
        &profile.provider_type,
        (!model.is_empty()).then_some(model),
        credential_scope,
        now,
    )
}

async fn fetch_gateway_json_once<T: DeserializeOwned>(
    client: &reqwest::Client,
    gateway: &GatewayEndpoint,
    path: &str,
    request_timeout: Duration,
    source_name: &str,
) -> Result<T, QuotaCheckError> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(client.get(url).timeout(request_timeout), gateway)
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                QuotaCheckError::TimedOut
            } else {
                QuotaCheckError::SourceUnavailable(format!("{source_name} request failed"))
            }
        })?;
    if !response.status().is_success() {
        return Err(QuotaCheckError::SourceUnavailable(format!(
            "{source_name} returned HTTP {}",
            response.status()
        )));
    }
    let bytes = response.bytes().await.map_err(|_| {
        QuotaCheckError::SourceUnavailable(format!("{source_name} response was unreadable"))
    })?;
    serde_json::from_slice(&bytes).map_err(|_| {
        QuotaCheckError::Indeterminate(format!(
            "{source_name} response did not match the expected schema"
        ))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskCreateQuotaExhausted {
    agent_id: String,
    provider_slug: String,
    scope: QuotaScope,
    reset_at: Option<String>,
}

impl TaskCreateQuotaExhausted {
    pub(super) fn from_status(agent_id: &str, status: QuotaStatus) -> Result<Self, QuotaStatus> {
        match status {
            QuotaStatus::Exhausted {
                provider,
                scope,
                reset_at,
            } => Ok(Self {
                agent_id: agent_id.to_owned(),
                provider_slug: provider.as_slug().to_owned(),
                scope,
                reset_at,
            }),
            other => Err(other),
        }
    }

    pub(crate) fn error_kind(&self) -> &'static str {
        "provider_quota_exhausted"
    }

    fn reset_suffix(&self) -> String {
        let Some(reset_at) = self.reset_at.as_deref() else {
            return String::new();
        };
        format!(" until {}", format_local_timestamp(Some(reset_at)))
    }
}

impl std::fmt::Display for TaskCreateQuotaExhausted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reset = self.reset_suffix();
        match &self.scope {
            QuotaScope::Window { name } => write!(
                formatter,
                "Agent '{}' uses provider '{}', whose '{}' quota window is exhausted{}; task was not created.",
                self.agent_id, self.provider_slug, name, reset
            ),
            QuotaScope::Model { name } => write!(
                formatter,
                "Agent '{}' uses provider '{}', whose quota for model '{}' is exhausted{}; task was not created.",
                self.agent_id, self.provider_slug, name, reset
            ),
        }
    }
}

impl std::error::Error for TaskCreateQuotaExhausted {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::task::{cmd_task_create, cmd_task_create_with_quota_timeout};
    use crate::commands::test_support::{RecordedRequest, write_test_gateway_config};
    use axum::{
        Json, Router,
        body::Body,
        extract::State,
        http::{StatusCode, header},
        response::{IntoResponse, Response},
        routing::{get, post},
    };
    use garyx_models::ProviderType;
    use serde_json::{Value, json};
    use std::future::pending;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;
    use tokio::{net::TcpListener, task::JoinHandle};

    #[derive(Clone)]
    enum MockReply {
        Json(StatusCode, Value),
        Raw(StatusCode, String),
        Pending,
    }

    #[derive(Clone)]
    struct MockState {
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
        profile: MockReply,
        usage: MockReply,
        task_status: StatusCode,
    }

    async fn mock_reply(reply: MockReply) -> Response {
        match reply {
            MockReply::Json(status, value) => (status, Json(value)).into_response(),
            MockReply::Raw(status, body) => Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .expect("raw response"),
            MockReply::Pending => pending::<Response>().await,
        }
    }

    async fn spawn_quota_task_server(state: MockState) -> (String, JoinHandle<()>) {
        let app = Router::new()
            .route(
                "/api/custom-agents/{agent_id}",
                get(|State(state): State<MockState>| async move {
                    state
                        .requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "GET".to_owned(),
                            path: "/api/custom-agents/test-agent".to_owned(),
                            body: Value::Null,
                        });
                    mock_reply(state.profile).await
                }),
            )
            .route(
                "/api/usage/coding",
                get(|State(state): State<MockState>| async move {
                    state
                        .requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "GET".to_owned(),
                            path: "/api/usage/coding".to_owned(),
                            body: Value::Null,
                        });
                    mock_reply(state.usage).await
                }),
            )
            .route(
                "/api/tasks",
                post(
                    |State(state): State<MockState>, Json(payload): Json<Value>| async move {
                        state
                            .requests
                            .lock()
                            .expect("request lock")
                            .push(RecordedRequest {
                                method: "POST".to_owned(),
                                path: "/api/tasks".to_owned(),
                                body: payload.clone(),
                            });
                        if state.task_status.is_success() {
                            (
                                state.task_status,
                                Json(json!({
                                    "thread_id": "thread::test-task",
                                    "task_id": "#TASK-1000000001",
                                    "number": 1000000001_u64,
                                    "status": "todo",
                                    "runtime_agent_id": "test-agent",
                                    "task": {
                                        "schema_version": "garyx.task.v1",
                                        "number": 1000000001_u64,
                                        "title": payload["title"],
                                        "status": "todo",
                                        "creator": {"kind": "human", "id": "cli"},
                                        "updated_by": {"kind": "human", "id": "cli"},
                                        "created_at": "2030-01-01T00:00:00Z",
                                        "updated_at": "2030-01-01T00:00:00Z",
                                        "events": []
                                    }
                                })),
                            )
                                .into_response()
                        } else {
                            (
                                state.task_status,
                                Json(json!({"error": "custom agent not found"})),
                            )
                                .into_response()
                        }
                    },
                ),
            )
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test router");
        });
        (format!("http://{addr}"), handle)
    }

    fn profile(provider_type: &str, model: &str, customized: bool) -> Value {
        json!({
            "agent_id": "test-agent",
            "display_name": "Test Agent",
            "provider_type": provider_type,
            "model": model,
            "provider_env": if customized { json!({"TEST_API_KEY": "${TOKEN}"}) } else { json!({}) },
            "system_prompt": "",
            "built_in": false,
            "standalone": true,
            "created_at": "2030-01-01T00:00:00Z",
            "updated_at": "2030-01-01T00:00:00Z"
        })
    }

    fn window_usage(id: &str, remaining: f64, stale: bool) -> Value {
        json!({
            "providers": [{
                "id": id,
                "available": true,
                "stale": stale,
                "session": {
                    "used_percent": 100.0 - remaining,
                    "remaining_percent": remaining,
                    "resets_at": "2030-01-02T12:00:00Z"
                }
            }],
            "refreshed_at": "2030-01-01T12:00:00Z"
        })
    }

    fn antigravity_usage() -> Value {
        json!({
            "providers": [{
                "id": "antigravity",
                "available": true,
                "models": [
                    {
                        "id": "model-a",
                        "name": "Model A",
                        "remaining_fraction": 0.0,
                        "remaining_percent": 0.0,
                        "used_percent": 100.0,
                        "resets_at": "2030-01-02T12:00:00Z"
                    },
                    {
                        "id": "model-b",
                        "name": "Model B",
                        "remaining_fraction": 0.5,
                        "remaining_percent": 50.0,
                        "used_percent": 50.0,
                        "resets_at": "2030-01-02T12:00:00Z"
                    }
                ]
            }],
            "refreshed_at": "2030-01-01T12:00:00Z"
        })
    }

    fn count_requests(records: &[RecordedRequest], method: &str, path: &str) -> usize {
        records
            .iter()
            .filter(|request| request.method == method && request.path == path)
            .count()
    }

    async fn run_task_create(
        profile_reply: MockReply,
        usage_reply: MockReply,
        task_status: StatusCode,
        timeout_override: Option<Duration>,
    ) -> (Result<(), Box<dyn std::error::Error>>, Vec<RecordedRequest>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = MockState {
            requests: requests.clone(),
            profile: profile_reply,
            usage: usage_reply,
            task_status,
        };
        let (base_url, handle) = spawn_quota_task_server(state).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);
        let args = (
            config_path.to_str().expect("config path"),
            Some("Quota task".to_owned()),
            Some("Synthetic test".to_owned()),
            Some("/tmp/test-repo".to_owned()),
            false,
            Some("test-agent".to_owned()),
            vec!["none".to_owned()],
        );
        let result = match timeout_override {
            Some(timeout) => {
                cmd_task_create_with_quota_timeout(
                    args.0, args.1, args.2, args.3, args.4, args.5, args.6, timeout,
                )
                .await
            }
            None => {
                cmd_task_create(args.0, args.1, args.2, args.3, args.4, args.5, args.6).await
            }
        };
        handle.abort();
        let records = requests.lock().expect("request lock").clone();
        (result, records)
    }

    #[test]
    fn exhausted_error_uses_canonical_provider_model_reset_and_required_phrase() {
        let error = TaskCreateQuotaExhausted::from_status(
            "Test Agent",
            QuotaStatus::Exhausted {
                provider: ProviderType::CodexAppServer,
                scope: QuotaScope::Model {
                    name: "test-model".to_owned(),
                },
                reset_at: Some("2030-01-02T12:00:00Z".to_owned()),
            },
        )
        .expect("exhausted status");
        let message = error.to_string();

        assert!(message.contains("Test Agent"));
        assert!(message.contains("codex_app_server"));
        assert!(message.contains("test-model"));
        assert!(message.contains("2030-01-02"));
        assert!(message.contains("task was not created"));
        let local_reset = message
            .split(" until ")
            .nth(1)
            .and_then(|suffix| suffix.split(';').next())
            .expect("local reset text");
        assert_eq!(local_reset.len(), "2030-01-02 12:00:00".len());
        assert_eq!(error.error_kind(), "provider_quota_exhausted");
    }

    #[tokio::test]
    async fn exhausted_quota_returns_typed_error_and_never_posts_task() {
        let (result, records) = run_task_create(
            MockReply::Json(StatusCode::OK, profile("codex_app_server", "", false)),
            MockReply::Json(StatusCode::OK, window_usage("codex", 0.0, false)),
            StatusCode::CREATED,
            None,
        )
        .await;
        let error = result.expect_err("exhausted quota should block");
        assert!(error.downcast_ref::<TaskCreateQuotaExhausted>().is_some());
        assert_eq!(count_requests(&records, "POST", "/api/tasks"), 0);
        assert_eq!(count_requests(&records, "GET", "/api/usage/coding"), 1);
    }

    #[tokio::test]
    async fn healthy_quota_posts_task_once() {
        let (result, records) = run_task_create(
            MockReply::Json(StatusCode::OK, profile("codex_app_server", "", false)),
            MockReply::Json(StatusCode::OK, window_usage("codex", 20.0, false)),
            StatusCode::CREATED,
            None,
        )
        .await;
        result.expect("healthy quota should allow task");
        assert_eq!(count_requests(&records, "POST", "/api/tasks"), 1);
    }

    #[tokio::test]
    async fn traex_reads_snapshot_then_silently_posts_as_unsupported() {
        let (result, records) = run_task_create(
            MockReply::Json(StatusCode::OK, profile("traex", "", false)),
            MockReply::Json(StatusCode::OK, window_usage("codex", 0.0, false)),
            StatusCode::CREATED,
            None,
        )
        .await;
        result.expect("unsupported provider should allow task");
        assert_eq!(count_requests(&records, "GET", "/api/usage/coding"), 1);
        assert_eq!(count_requests(&records, "POST", "/api/tasks"), 1);
    }

    #[tokio::test]
    async fn customized_credentials_skip_usage_and_post_task() {
        let (result, records) = run_task_create(
            MockReply::Json(StatusCode::OK, profile("codex_app_server", "", true)),
            MockReply::Pending,
            StatusCode::CREATED,
            None,
        )
        .await;
        result.expect("credential mismatch should fail open");
        assert_eq!(count_requests(&records, "GET", "/api/usage/coding"), 0);
        assert_eq!(count_requests(&records, "POST", "/api/tasks"), 1);
    }

    #[tokio::test]
    async fn usage_failures_and_stale_zero_all_fail_open_to_one_post() {
        let cases = [
            MockReply::Json(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": "synthetic failure"}),
            ),
            MockReply::Raw(StatusCode::OK, "{not-json".to_owned()),
            MockReply::Json(StatusCode::OK, window_usage("codex", 0.0, true)),
        ];
        for usage in cases {
            let (result, records) = run_task_create(
                MockReply::Json(StatusCode::OK, profile("codex_app_server", "", false)),
                usage,
                StatusCode::CREATED,
                None,
            )
            .await;
            result.expect("indeterminate usage should fail open");
            assert_eq!(count_requests(&records, "POST", "/api/tasks"), 1);
        }
    }

    #[tokio::test]
    async fn short_injected_timeout_fails_open_without_waiting_seconds() {
        let (result, records) = run_task_create(
            MockReply::Json(StatusCode::OK, profile("codex_app_server", "", false)),
            MockReply::Pending,
            StatusCode::CREATED,
            Some(Duration::from_millis(5)),
        )
        .await;
        result.expect("timeout should fail open");
        assert_eq!(count_requests(&records, "POST", "/api/tasks"), 1);
    }

    #[tokio::test]
    async fn missing_agent_profile_still_reaches_existing_task_validation() {
        let (result, records) = run_task_create(
            MockReply::Json(
                StatusCode::NOT_FOUND,
                json!({"error": "custom agent not found"}),
            ),
            MockReply::Pending,
            StatusCode::BAD_REQUEST,
            None,
        )
        .await;
        result.expect_err("task route should retain its existing rejection");
        assert_eq!(count_requests(&records, "GET", "/api/usage/coding"), 0);
        assert_eq!(count_requests(&records, "POST", "/api/tasks"), 1);
    }

    #[tokio::test]
    async fn antigravity_checks_only_the_selected_model_bucket() {
        let (healthy_result, healthy_records) = run_task_create(
            MockReply::Json(StatusCode::OK, profile("antigravity", "model-b", false)),
            MockReply::Json(StatusCode::OK, antigravity_usage()),
            StatusCode::CREATED,
            None,
        )
        .await;
        healthy_result.expect("selected healthy bucket should allow task");
        assert_eq!(count_requests(&healthy_records, "POST", "/api/tasks"), 1);

        let (exhausted_result, exhausted_records) = run_task_create(
            MockReply::Json(StatusCode::OK, profile("antigravity", "model-a", false)),
            MockReply::Json(StatusCode::OK, antigravity_usage()),
            StatusCode::CREATED,
            None,
        )
        .await;
        exhausted_result.expect_err("selected exhausted bucket should block task");
        assert_eq!(count_requests(&exhausted_records, "POST", "/api/tasks"), 0);
    }
}
