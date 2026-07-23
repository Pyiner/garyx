use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::stream::{self, StreamExt};
use garyx_models::config::ApnsConfig;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Mutex;
use tracing::{error, warn};

use crate::garyx_db::{GaryxDbError, GaryxDbService, PushDeviceToken, PushDeviceTokenDraft};
use crate::server::AppState;

const APNS_JWT_CACHE_TTL: Duration = Duration::from_secs(50 * 60);
const APNS_EXPIRATION_TTL_SECS: u64 = 24 * 60 * 60;
const APNS_MAX_ATTEMPTS: usize = 3;
const APNS_CONCURRENCY: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApnsTransportRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl ApnsTransportRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApnsTransportResponse {
    pub status: u16,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("{message}")]
pub struct ApnsTransportError {
    message: String,
}

impl ApnsTransportError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait ApnsTransport: Send + Sync {
    async fn send(
        &self,
        request: ApnsTransportRequest,
    ) -> Result<ApnsTransportResponse, ApnsTransportError>;
}

/// The single production transport. Garyx already standardizes on reqwest,
/// so this keeps APNs on the existing HTTP stack while explicitly requiring
/// HTTP/2 instead of introducing the additional transport stack used by `a2`.
pub struct ReqwestApnsTransport {
    client: reqwest::Client,
}

impl ReqwestApnsTransport {
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .http2_adaptive_window(true)
            .build()
            .map_err(|error| format!("failed to initialize APNs HTTP/2 client: {error}"))?;
        Ok(Self { client })
    }
}

#[derive(Deserialize)]
struct ApnsErrorPayload {
    reason: Option<String>,
}

#[async_trait]
impl ApnsTransport for ReqwestApnsTransport {
    async fn send(
        &self,
        request: ApnsTransportRequest,
    ) -> Result<ApnsTransportResponse, ApnsTransportError> {
        let mut builder = self
            .client
            .post(&request.url)
            .version(reqwest::Version::HTTP_2)
            .body(request.body);
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| ApnsTransportError::new(error.to_string()))?;
        let status = response.status().as_u16();
        let reason = response
            .json::<ApnsErrorPayload>()
            .await
            .ok()
            .and_then(|payload| payload.reason);
        Ok(ApnsTransportResponse { status, reason })
    }
}

#[derive(Serialize)]
struct ApnsJwtClaims<'a> {
    iss: &'a str,
    iat: u64,
}

struct CachedJwt {
    token: String,
    created_at: Instant,
}

struct ApnsClient {
    transport: Arc<dyn ApnsTransport>,
    encoding_key: EncodingKey,
    key_id: String,
    team_id: String,
    cached_jwt: Mutex<Option<CachedJwt>>,
}

impl ApnsClient {
    fn from_config(config: &ApnsConfig, transport: Arc<dyn ApnsTransport>) -> Result<Self, String> {
        let key_id = require_config_field("key_id", &config.key_id)?;
        let team_id = require_config_field("team_id", &config.team_id)?;
        require_config_field("topic", &config.topic)?;
        let key_path = require_config_field("key_path", &config.key_path)?;
        let key_bytes = std::fs::read(&key_path)
            .map_err(|error| format!("failed to read APNs key '{}': {error}", key_path))?;
        let encoding_key = EncodingKey::from_ec_pem(&key_bytes)
            .map_err(|error| format!("invalid APNs ES256 key '{}': {error}", key_path))?;
        Ok(Self {
            transport,
            encoding_key,
            key_id,
            team_id,
            cached_jwt: Mutex::new(None),
        })
    }

    async fn bearer_token(&self) -> Result<String, String> {
        let mut cached = self.cached_jwt.lock().await;
        if let Some(current) = cached.as_ref()
            && current.created_at.elapsed() < APNS_JWT_CACHE_TTL
        {
            return Ok(current.token.clone());
        }
        let issued_at = unix_timestamp()?;
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.key_id.clone());
        let token = jsonwebtoken::encode(
            &header,
            &ApnsJwtClaims {
                iss: &self.team_id,
                iat: issued_at,
            },
            &self.encoding_key,
        )
        .map_err(|error| format!("failed to create APNs JWT: {error}"))?;
        *cached = Some(CachedJwt {
            token: token.clone(),
            created_at: Instant::now(),
        });
        Ok(token)
    }
}

fn require_config_field(field: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        Err(format!("push.apns.{field} must not be empty"))
    } else {
        Ok(value.to_owned())
    }
}

fn unix_timestamp() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterPushDeviceRequest {
    pub token: String,
    pub platform: String,
    pub environment: String,
    pub bundle_id: String,
    pub device_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendPushRequest {
    pub title: String,
    pub body: String,
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SendPushResponse {
    pub sent: usize,
    pub failed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_devices: Option<bool>,
}

#[derive(Serialize)]
struct ApnsPayload<'a> {
    aps: ApsPayload<'a>,
    garyx: GaryxPayload<'a>,
}

#[derive(Serialize)]
struct ApsPayload<'a> {
    alert: AlertPayload<'a>,
    sound: &'static str,
    #[serde(rename = "thread-id", skip_serializing_if = "Option::is_none")]
    thread_id: Option<&'a str>,
}

#[derive(Serialize)]
struct AlertPayload<'a> {
    title: &'a str,
    body: &'a str,
}

#[derive(Serialize)]
struct GaryxPayload<'a> {
    v: u8,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id: Option<&'a str>,
}

pub(crate) struct PushNotificationService {
    db: Arc<GaryxDbService>,
    apns: ApnsClient,
    configured_topic: String,
    retry_delay: Duration,
}

impl PushNotificationService {
    pub(crate) fn from_config(
        db: Arc<GaryxDbService>,
        config: &ApnsConfig,
        transport: Arc<dyn ApnsTransport>,
    ) -> Result<Self, String> {
        let configured_topic = require_config_field("topic", &config.topic)?;
        Ok(Self {
            db,
            apns: ApnsClient::from_config(config, transport)?,
            configured_topic,
            retry_delay: Duration::from_millis(150),
        })
    }

    pub(crate) async fn send_manual(
        &self,
        request: SendPushRequest,
    ) -> Result<SendPushResponse, PushSendError> {
        let title = normalize_request_field("title", &request.title)?;
        let body = normalize_request_field("body", &request.body)?;
        let thread_id = request
            .thread_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let devices = self
            .db
            .clone()
            .run_blocking(|db| db.list_push_device_tokens())
            .await
            .map_err(|error| PushSendError::Storage(error.to_string()))?;
        if devices.is_empty() {
            return Ok(SendPushResponse {
                sent: 0,
                failed: 0,
                no_devices: Some(true),
            });
        }

        let outcomes = stream::iter(devices.into_iter().map(|device| {
            let title = title.clone();
            let body = body.clone();
            let thread_id = thread_id.clone();
            async move {
                self.deliver_to_device(&device, &title, &body, thread_id.as_deref())
                    .await
            }
        }))
        .buffer_unordered(APNS_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
        let sent = outcomes.iter().filter(|outcome| **outcome).count();
        Ok(SendPushResponse {
            sent,
            failed: outcomes.len().saturating_sub(sent),
            no_devices: None,
        })
    }

    async fn deliver_to_device(
        &self,
        device: &PushDeviceToken,
        title: &str,
        body: &str,
        thread_id: Option<&str>,
    ) -> bool {
        if device.bundle_id != self.configured_topic {
            warn!(
                registered_bundle_id = %device.bundle_id,
                configured_topic = %self.configured_topic,
                "skipping APNs token registered for a different bundle"
            );
            return false;
        }

        let payload = ApnsPayload {
            aps: ApsPayload {
                alert: AlertPayload { title, body },
                sound: "default",
                thread_id,
            },
            garyx: GaryxPayload {
                v: 1,
                kind: "manual",
                thread_id,
            },
        };
        let body = match serde_json::to_vec(&payload) {
            Ok(body) => body,
            Err(error) => {
                error!(error = %error, "failed to serialize APNs payload");
                return false;
            }
        };

        for attempt in 1..=APNS_MAX_ATTEMPTS {
            let request = match self
                .build_transport_request(device, body.clone(), thread_id)
                .await
            {
                Ok(request) => request,
                Err(error) => {
                    error!(error = %error, "failed to prepare APNs request");
                    return false;
                }
            };
            match self.apns.transport.send(request).await {
                Ok(response) if (200..300).contains(&response.status) => return true,
                Ok(response) if should_delete_token(&response) => {
                    let token = device.token.clone();
                    if let Err(error) = self
                        .db
                        .clone()
                        .run_blocking(move |db| db.delete_push_device_token(&token))
                        .await
                    {
                        error!(error = %error, "failed to delete an invalid APNs token");
                    }
                    return false;
                }
                Ok(response)
                    if is_retryable_status(response.status) && attempt < APNS_MAX_ATTEMPTS =>
                {
                    tokio::time::sleep(self.retry_delay * attempt as u32).await;
                }
                Ok(response) => {
                    warn!(
                        status = response.status,
                        reason = response.reason.as_deref().unwrap_or(""),
                        "APNs rejected a notification"
                    );
                    return false;
                }
                Err(error) if attempt < APNS_MAX_ATTEMPTS => {
                    warn!(
                        attempt,
                        error = %error,
                        "APNs network request failed; retrying"
                    );
                    tokio::time::sleep(self.retry_delay * attempt as u32).await;
                }
                Err(error) => {
                    warn!(error = %error, "APNs network request failed");
                    return false;
                }
            }
        }
        false
    }

    async fn build_transport_request(
        &self,
        device: &PushDeviceToken,
        body: Vec<u8>,
        thread_id: Option<&str>,
    ) -> Result<ApnsTransportRequest, String> {
        let host = match device.environment.as_str() {
            "development" => "api.sandbox.push.apple.com",
            "production" => "api.push.apple.com",
            other => return Err(format!("unsupported APNs environment '{other}'")),
        };
        let bearer = self.apns.bearer_token().await?;
        let expiration = unix_timestamp()?.saturating_add(APNS_EXPIRATION_TTL_SECS);
        let mut headers = vec![
            ("authorization".to_owned(), format!("bearer {bearer}")),
            ("apns-push-type".to_owned(), "alert".to_owned()),
            ("apns-priority".to_owned(), "10".to_owned()),
            ("apns-topic".to_owned(), device.bundle_id.clone()),
            ("apns-expiration".to_owned(), expiration.to_string()),
            ("content-type".to_owned(), "application/json".to_owned()),
        ];
        if let Some(thread_id) = thread_id {
            headers.push(("apns-collapse-id".to_owned(), thread_id.to_owned()));
        }
        Ok(ApnsTransportRequest {
            url: format!(
                "https://{host}/3/device/{}",
                urlencoding::encode(&device.token)
            ),
            headers,
            body,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PushSendError {
    #[error("{0}")]
    InvalidRequest(String),
    #[error("{0}")]
    Storage(String),
}

fn normalize_request_field(field: &str, value: &str) -> Result<String, PushSendError> {
    if value.trim().is_empty() {
        Err(PushSendError::InvalidRequest(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(value.to_owned())
    }
}

fn should_delete_token(response: &ApnsTransportResponse) -> bool {
    response.status == 410
        || (response.status == 400 && response.reason.as_deref() == Some("BadDeviceToken"))
}

fn is_retryable_status(status: u16) -> bool {
    status == 429 || status >= 500
}

#[derive(Debug)]
pub(crate) struct PushApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl PushApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_push_request",
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "push_storage_failed",
            message: message.into(),
        }
    }

    fn disabled() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "push_disabled",
            message: "APNs push is not configured or failed to initialize".to_owned(),
        }
    }
}

impl IntoResponse for PushApiError {
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

fn map_db_error(error: GaryxDbError) -> PushApiError {
    match error {
        GaryxDbError::BadRequest(message) => PushApiError::bad_request(message),
        other => PushApiError::internal(other.to_string()),
    }
}

pub(crate) async fn register_device(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RegisterPushDeviceRequest>,
) -> Result<StatusCode, PushApiError> {
    state
        .ops
        .garyx_db
        .clone()
        .run_blocking(move |db| {
            db.upsert_push_device_token(PushDeviceTokenDraft {
                token: &request.token,
                platform: &request.platform,
                environment: &request.environment,
                bundle_id: &request.bundle_id,
                device_name: request.device_name.as_deref(),
            })
        })
        .await
        .map_err(map_db_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn delete_device(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Result<StatusCode, PushApiError> {
    state
        .ops
        .garyx_db
        .clone()
        .run_blocking(move |db| db.delete_push_device_token(&token))
        .await
        .map_err(map_db_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn send_manual(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SendPushRequest>,
) -> Result<Json<SendPushResponse>, PushApiError> {
    let service = state
        .ops
        .push_notifications
        .as_ref()
        .ok_or_else(PushApiError::disabled)?;
    let response = service
        .send_manual(request)
        .await
        .map_err(|error| match error {
            PushSendError::InvalidRequest(message) => PushApiError::bad_request(message),
            PushSendError::Storage(message) => PushApiError::internal(message),
        })?;
    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::collections::VecDeque;
    use std::sync::Mutex as StdMutex;

    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use base64::Engine;
    use garyx_models::config::{GaryxConfig, PushConfig};
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::route_graph::build_router;
    use crate::server::AppStateBuilder;
    use crate::test_support::{authed_request, with_gateway_auth};

    const TEST_PRIVATE_KEY: &[u8] = include_bytes!("fixtures/apns-test-key.p8");

    #[derive(Default)]
    struct MockApnsTransport {
        requests: StdMutex<Vec<ApnsTransportRequest>>,
        responses_by_token: StdMutex<HashMap<String, VecDeque<ApnsTransportResponse>>>,
    }

    impl MockApnsTransport {
        fn respond_for(&self, token: &str, responses: Vec<ApnsTransportResponse>) {
            self.responses_by_token
                .lock()
                .unwrap()
                .insert(token.to_owned(), responses.into());
        }

        fn requests(&self) -> Vec<ApnsTransportRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ApnsTransport for MockApnsTransport {
        async fn send(
            &self,
            request: ApnsTransportRequest,
        ) -> Result<ApnsTransportResponse, ApnsTransportError> {
            let token = request
                .url
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_owned();
            self.requests.lock().unwrap().push(request);
            Ok(self
                .responses_by_token
                .lock()
                .unwrap()
                .get_mut(&token)
                .and_then(VecDeque::pop_front)
                .unwrap_or(ApnsTransportResponse {
                    status: 200,
                    reason: None,
                }))
        }
    }

    struct TestPushApp {
        router: axum::Router,
        db: Arc<GaryxDbService>,
        transport: Arc<MockApnsTransport>,
        _temp: tempfile::TempDir,
    }

    fn test_push_app() -> TestPushApp {
        let temp = tempfile::tempdir().unwrap();
        let key_path = temp.path().join("synthetic-apns-key.p8");
        std::fs::write(&key_path, TEST_PRIVATE_KEY).unwrap();
        let mut config = with_gateway_auth(GaryxConfig::default());
        config.push = Some(PushConfig {
            apns: ApnsConfig {
                key_path: key_path.to_string_lossy().into_owned(),
                key_id: "TESTKEY123".to_owned(),
                team_id: "TESTTEAM12".to_owned(),
                topic: "com.garyx.mobile".to_owned(),
            },
        });
        let db = Arc::new(GaryxDbService::memory().unwrap());
        let transport = Arc::new(MockApnsTransport::default());
        let state = AppStateBuilder::new(config)
            .with_garyx_db(db.clone())
            .with_apns_transport(transport.clone())
            .build();
        TestPushApp {
            router: build_router(state),
            db,
            transport,
            _temp: temp,
        }
    }

    fn insert_device(db: &GaryxDbService, token: &str, environment: &str) {
        db.upsert_push_device_token(PushDeviceTokenDraft {
            token,
            platform: "ios",
            environment,
            bundle_id: "com.garyx.mobile",
            device_name: None,
        })
        .unwrap();
    }

    async fn json_response(response: Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn send_request(body: Value) -> Request<Body> {
        authed_request()
            .method(Method::POST)
            .uri("/api/push/send")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[test]
    fn jwt_is_es256_and_contains_apple_claims() {
        let temp = tempfile::tempdir().unwrap();
        let key_path = temp.path().join("synthetic-apns-key.p8");
        std::fs::write(&key_path, TEST_PRIVATE_KEY).unwrap();
        let client = ApnsClient::from_config(
            &ApnsConfig {
                key_path: key_path.to_string_lossy().into_owned(),
                key_id: "TESTKEY123".to_owned(),
                team_id: "TESTTEAM12".to_owned(),
                topic: "com.garyx.mobile".to_owned(),
            },
            Arc::new(MockApnsTransport::default()),
        )
        .unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let first = runtime.block_on(client.bearer_token()).unwrap();
        let second = runtime.block_on(client.bearer_token()).unwrap();
        assert_eq!(first, second, "JWT must be reused inside the cache window");
        let header = jsonwebtoken::decode_header(&first).unwrap();
        assert_eq!(header.alg, Algorithm::ES256);
        assert_eq!(header.kid.as_deref(), Some("TESTKEY123"));
        let payload = first.split('.').nth(1).unwrap();
        let claims: Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(claims["iss"], "TESTTEAM12");
        assert!(claims["iat"].as_u64().is_some());
    }

    #[tokio::test]
    async fn device_routes_upsert_and_delete_idempotently() {
        let app = test_push_app();
        let register = |environment: &str, name: Option<&str>| {
            let mut body = json!({
                "token": "synthetic-device-token",
                "platform": "ios",
                "environment": environment,
                "bundle_id": "com.garyx.mobile"
            });
            if let Some(name) = name {
                body["device_name"] = Value::String(name.to_owned());
            }
            authed_request()
                .method(Method::POST)
                .uri("/api/push/devices")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        };
        let response = app
            .router
            .clone()
            .oneshot(register("development", Some("Test iPhone")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let registered_at = app.db.list_push_device_tokens().unwrap()[0]
            .registered_at
            .clone();
        let first_last_seen_at = app.db.list_push_device_tokens().unwrap()[0]
            .last_seen_at
            .clone();
        tokio::time::sleep(Duration::from_millis(2)).await;

        let response = app
            .router
            .clone()
            .oneshot(register("production", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let rows = app.db.list_push_device_tokens().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].environment, "production");
        assert_eq!(rows[0].registered_at, registered_at);
        assert!(rows[0].last_seen_at > first_last_seen_at);

        let response = app
            .router
            .oneshot(
                authed_request()
                    .method(Method::DELETE)
                    .uri("/api/push/devices/synthetic-device-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(app.db.list_push_device_tokens().unwrap().is_empty());
    }

    #[tokio::test]
    async fn push_routes_require_gateway_authentication() {
        let app = test_push_app();
        for request in [
            Request::builder()
                .method(Method::POST)
                .uri("/api/push/devices")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "token": "synthetic-device-token",
                        "platform": "ios",
                        "environment": "development",
                        "bundle_id": "com.garyx.mobile"
                    })
                    .to_string(),
                ))
                .unwrap(),
            Request::builder()
                .method(Method::DELETE)
                .uri("/api/push/devices/synthetic-device-token")
                .body(Body::empty())
                .unwrap(),
            Request::builder()
                .method(Method::POST)
                .uri("/api/push/send")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"title": "Hello", "body": "World"}).to_string(),
                ))
                .unwrap(),
        ] {
            let response = app.router.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn send_is_disabled_without_push_configuration() {
        let config = with_gateway_auth(GaryxConfig::default());
        let state = AppStateBuilder::new(config).build();
        let response = build_router(state)
            .oneshot(send_request(json!({"title": "Hello", "body": "World"})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            json_response(response).await,
            json!({
                "error": {
                    "code": "push_disabled",
                    "message": "APNs push is not configured or failed to initialize"
                }
            })
        );
    }

    #[tokio::test]
    async fn send_returns_successful_empty_delivery_when_no_devices_exist() {
        let app = test_push_app();
        let response = app
            .router
            .oneshot(send_request(json!({"title": "Hello", "body": "World"})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            json_response(response).await,
            json!({"sent": 0, "failed": 0, "no_devices": true})
        );
        assert!(app.transport.requests().is_empty());
    }

    #[tokio::test]
    async fn send_delivers_to_all_devices_and_builds_thread_payload_and_headers() {
        let app = test_push_app();
        insert_device(&app.db, "development-token", "development");
        insert_device(&app.db, "production-token", "production");
        let response = app
            .router
            .clone()
            .oneshot(send_request(json!({
                "title": "Run finished",
                "body": "Open the thread",
                "thread_id": "thread::synthetic"
            })))
            .await
            .unwrap();
        assert_eq!(
            json_response(response).await,
            json!({"sent": 2, "failed": 0})
        );
        let requests = app.transport.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().any(|request| {
            request
                .url
                .starts_with("https://api.sandbox.push.apple.com/")
        }));
        assert!(
            requests
                .iter()
                .any(|request| request.url.starts_with("https://api.push.apple.com/"))
        );
        for request in requests {
            assert_eq!(
                request.header("apns-collapse-id"),
                Some("thread::synthetic")
            );
            assert_eq!(request.header("apns-push-type"), Some("alert"));
            assert_eq!(request.header("apns-priority"), Some("10"));
            assert_eq!(request.header("apns-topic"), Some("com.garyx.mobile"));
            let payload: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(payload["aps"]["thread-id"], "thread::synthetic");
            assert_eq!(payload["garyx"]["v"], 1);
            assert_eq!(payload["garyx"]["kind"], "manual");
            assert_eq!(payload["garyx"]["thread_id"], "thread::synthetic");
        }
    }

    #[tokio::test]
    async fn send_without_thread_id_omits_thread_and_collapse_fields() {
        let app = test_push_app();
        insert_device(&app.db, "production-token", "production");
        let response = app
            .router
            .clone()
            .oneshot(send_request(json!({"title": "Hello", "body": "World"})))
            .await
            .unwrap();
        assert_eq!(
            json_response(response).await,
            json!({"sent": 1, "failed": 0})
        );
        let request = app.transport.requests().remove(0);
        assert_eq!(request.header("apns-collapse-id"), None);
        let payload: Value = serde_json::from_slice(&request.body).unwrap();
        assert!(payload["aps"].get("thread-id").is_none());
        assert!(payload["garyx"].get("thread_id").is_none());
        assert_eq!(payload["garyx"]["kind"], "manual");
    }

    #[tokio::test]
    async fn send_isolates_partial_failure_and_retries_boundedly() {
        let app = test_push_app();
        insert_device(&app.db, "healthy-token", "production");
        insert_device(&app.db, "failing-token", "production");
        app.transport.respond_for(
            "failing-token",
            vec![
                ApnsTransportResponse {
                    status: 503,
                    reason: Some("ServiceUnavailable".to_owned()),
                },
                ApnsTransportResponse {
                    status: 503,
                    reason: Some("ServiceUnavailable".to_owned()),
                },
                ApnsTransportResponse {
                    status: 503,
                    reason: Some("ServiceUnavailable".to_owned()),
                },
            ],
        );
        let response = app
            .router
            .clone()
            .oneshot(send_request(json!({"title": "Hello", "body": "World"})))
            .await
            .unwrap();
        assert_eq!(
            json_response(response).await,
            json!({"sent": 1, "failed": 1})
        );
        let requests = app.transport.requests();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.url.ends_with("/failing-token"))
                .count(),
            APNS_MAX_ATTEMPTS
        );
    }

    #[tokio::test]
    async fn invalid_apns_responses_synchronously_remove_tokens() {
        let app = test_push_app();
        insert_device(&app.db, "unregistered-token", "production");
        insert_device(&app.db, "bad-token", "production");
        app.transport.respond_for(
            "unregistered-token",
            vec![ApnsTransportResponse {
                status: 410,
                reason: Some("Unregistered".to_owned()),
            }],
        );
        app.transport.respond_for(
            "bad-token",
            vec![ApnsTransportResponse {
                status: 400,
                reason: Some("BadDeviceToken".to_owned()),
            }],
        );
        let response = app
            .router
            .clone()
            .oneshot(send_request(json!({"title": "Hello", "body": "World"})))
            .await
            .unwrap();
        assert_eq!(
            json_response(response).await,
            json!({"sent": 0, "failed": 2})
        );
        assert!(app.db.list_push_device_tokens().unwrap().is_empty());
    }
}
