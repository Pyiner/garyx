use super::*;

use std::sync::OnceLock;
use std::time::Duration;

#[derive(Debug, Clone)]
pub(super) struct GatewayEndpoint {
    pub(super) base_url: String,
    pub(super) auth_token: Option<String>,
}

/// Per-attempt timeout for idempotent (GET) gateway requests.
const GATEWAY_GET_TIMEOUT: Duration = Duration::from_secs(5);
/// Per-attempt timeout for mutating gateway requests.
const GATEWAY_MUTATION_TIMEOUT: Duration = Duration::from_secs(10);
/// Backoff before the second and third attempts.
const GATEWAY_RETRY_BACKOFFS: [Duration; 2] =
    [Duration::from_millis(250), Duration::from_millis(750)];
/// Total attempts allowed when the failure class is connect (no listener —
/// fails fast, so retrying is cheap and covers the gateway restart window).
const GATEWAY_MAX_CONNECT_ATTEMPTS: usize = 3;
/// Total attempts that may each burn a full per-attempt timeout. Only
/// idempotent requests retry timeouts, and only once, so a stalled control
/// plane costs at most ~2x the single-shot budget.
const GATEWAY_MAX_TIMEOUT_ATTEMPTS: usize = 2;

/// One shared HTTP client for every gateway-backed CLI command: keeps
/// connection pooling instead of paying TLS/TCP setup per helper call.
fn shared_http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// Retry classes for gateway JSON requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GatewayRetryPolicy {
    /// Safe to resend after connect failures and timeouts (GET).
    Idempotent,
    /// Resend only when the connection was refused, meaning the request
    /// never reached a listener. A timed-out mutation may have landed and
    /// must not be resent blindly.
    ConnectOnly,
}

/// Pure retry decision so the policy table is unit-testable without a
/// network: `attempts_made`/`timeout_attempts_made` count attempts already
/// sent, including the one that just failed.
fn transport_failure_retriable(
    policy: GatewayRetryPolicy,
    is_connect: bool,
    is_timeout: bool,
    attempts_made: usize,
    timeout_attempts_made: usize,
) -> bool {
    if attempts_made >= GATEWAY_MAX_CONNECT_ATTEMPTS {
        return false;
    }
    if is_connect {
        return true;
    }
    policy == GatewayRetryPolicy::Idempotent
        && is_timeout
        && timeout_attempts_made < GATEWAY_MAX_TIMEOUT_ATTEMPTS
}

fn retry_backoff(attempts_made: usize) -> Duration {
    let index = attempts_made
        .saturating_sub(1)
        .min(GATEWAY_RETRY_BACKOFFS.len() - 1);
    GATEWAY_RETRY_BACKOFFS[index]
}

pub(super) fn gateway_endpoint(
    config_path: &str,
) -> Result<GatewayEndpoint, Box<dyn std::error::Error>> {
    let config = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?.config;
    let public_url = config.gateway.public_url.trim();
    let base_url = if !public_url.is_empty() {
        public_url.trim_end_matches('/').to_owned()
    } else {
        let host = if config.gateway.host == "0.0.0.0" {
            "127.0.0.1".to_owned()
        } else {
            config.gateway.host
        };
        format!("http://{}:{}", host, config.gateway.port)
    };
    let auth_token = (!config.gateway.auth_token.trim().is_empty())
        .then(|| config.gateway.auth_token.trim().to_owned());
    Ok(GatewayEndpoint {
        base_url,
        auth_token,
    })
}

#[cfg(test)]
fn gateway_base_url(config_path: &str) -> Result<String, Box<dyn std::error::Error>> {
    Ok(gateway_endpoint(config_path)?.base_url)
}

pub(super) fn gateway_request(
    mut builder: reqwest::RequestBuilder,
    gateway: &GatewayEndpoint,
) -> reqwest::RequestBuilder {
    if let Some(token) = gateway.auth_token.as_deref() {
        builder = builder.bearer_auth(token);
    }
    builder
}

/// Failure classes for gateway-backed CLI commands.
///
/// The class drives the process exit code (see `cli_error_exit_code` in
/// `main.rs`) and the `error.kind` field of the `--json` failure envelope, so
/// scripts and agents can react without parsing message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GatewayErrorKind {
    /// The gateway did not answer at all (connect refused / timeout).
    Unreachable,
    /// The gateway answered 404 for the addressed resource.
    NotFound,
    /// The gateway answered 409: the resource changed concurrently (or a
    /// create hit an existing id). Re-read and retry.
    Conflict,
    /// The gateway rejected the request with any other non-success status.
    Rejected,
}

impl GatewayErrorKind {
    pub(crate) fn slug(self) -> &'static str {
        match self {
            GatewayErrorKind::Unreachable => "gateway_unreachable",
            GatewayErrorKind::NotFound => "not_found",
            GatewayErrorKind::Conflict => "conflict",
            GatewayErrorKind::Rejected => "gateway_rejected",
        }
    }
}

/// Structured error for gateway-backed commands. `Display` is a single
/// human-readable line (no nested Debug dumps, no double-encoded JSON).
#[derive(Debug)]
pub(crate) struct GatewayCliError {
    pub(crate) kind: GatewayErrorKind,
    pub(crate) message: String,
}

impl std::fmt::Display for GatewayCliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GatewayCliError {}

pub(super) fn gateway_send_error(
    gateway: &GatewayEndpoint,
    error: &reqwest::Error,
) -> GatewayCliError {
    if error.is_timeout() {
        // Distinct from connect-refused: the gateway is up but slow (e.g. a
        // provider bridge reload after an agent/team mutation). "Is it
        // running?" would mislead here.
        GatewayCliError {
            kind: GatewayErrorKind::Unreachable,
            message: format!(
                "gateway at {} did not respond in time — it may be busy; retry, or check `garyx status` / `garyx logs tail`",
                gateway.base_url
            ),
        }
    } else if error.is_connect() {
        GatewayCliError {
            kind: GatewayErrorKind::Unreachable,
            message: format!(
                "gateway not reachable at {} — is it running? Check `garyx status`; start it with `garyx gateway start` (or `garyx gateway install` on first setup)",
                gateway.base_url
            ),
        }
    } else {
        GatewayCliError {
            kind: GatewayErrorKind::Rejected,
            message: format!("gateway request failed: {error}"),
        }
    }
}

fn gateway_status_error(status: reqwest::StatusCode, body: &str) -> GatewayCliError {
    // Prefer the gateway's own `{"error": "..."}` detail over echoing the raw
    // JSON body, which double-encodes when the whole error is stringified.
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| body.trim().to_owned());
    let kind = if status == reqwest::StatusCode::NOT_FOUND {
        GatewayErrorKind::NotFound
    } else if status == reqwest::StatusCode::CONFLICT {
        GatewayErrorKind::Conflict
    } else {
        GatewayErrorKind::Rejected
    };
    let message = if detail.is_empty() {
        format!("gateway request failed: {status}")
    } else {
        format!("gateway request failed: {status}: {detail}")
    };
    GatewayCliError { kind, message }
}

/// Send a prepared gateway request and decode the JSON response.
///
/// Single choke point for every gateway JSON helper below: retries
/// transient transport failures per `policy` with short backoff, classifies
/// connect/timeout failures as `Unreachable`, non-2xx statuses as
/// `NotFound`/`Rejected`, and treats an empty success body as `{}`.
async fn execute_gateway_json(
    builder: reqwest::RequestBuilder,
    gateway: &GatewayEndpoint,
    policy: GatewayRetryPolicy,
) -> Result<Value, Box<dyn std::error::Error>> {
    let builder = gateway_request(builder, gateway);
    if builder.try_clone().is_none() {
        // Non-clonable (streaming) body: single shot, previous behavior.
        let response = builder
            .send()
            .await
            .map_err(|error| gateway_send_error_for_policy(gateway, &error, policy))?;
        return decode_gateway_response(response).await;
    }

    let mut attempts_made = 0usize;
    let mut timeout_attempts_made = 0usize;
    let response = loop {
        let attempt = builder
            .try_clone()
            .ok_or("gateway request body is not retriable")?;
        attempts_made += 1;
        match attempt.send().await {
            Ok(response) => break response,
            Err(error) => {
                if error.is_timeout() {
                    timeout_attempts_made += 1;
                }
                if !transport_failure_retriable(
                    policy,
                    error.is_connect(),
                    error.is_timeout(),
                    attempts_made,
                    timeout_attempts_made,
                ) {
                    return Err(gateway_send_error_for_policy(gateway, &error, policy).into());
                }
                tokio::time::sleep(retry_backoff(attempts_made)).await;
            }
        }
    };
    decode_gateway_response(response).await
}

async fn decode_gateway_response(
    response: reqwest::Response,
) -> Result<Value, Box<dyn std::error::Error>> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(gateway_status_error(status, &body).into());
    }
    if body.trim().is_empty() {
        return Ok(json!({}));
    }
    Ok(serde_json::from_str(&body)?)
}

/// Mutations that time out are never auto-resent; tell the caller to verify
/// whether the write landed before retrying by hand.
fn gateway_send_error_for_policy(
    gateway: &GatewayEndpoint,
    error: &reqwest::Error,
    policy: GatewayRetryPolicy,
) -> GatewayCliError {
    let mut cli_error = gateway_send_error(gateway, error);
    if policy == GatewayRetryPolicy::ConnectOnly && error.is_timeout() && !error.is_connect() {
        cli_error.message.push_str(
            " — the request was not auto-retried; verify whether the write landed (e.g. `garyx task list`) before resending",
        );
    }
    cli_error
}

pub(super) async fn fetch_gateway_json(
    gateway: &GatewayEndpoint,
    path_and_query: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path_and_query);
    let builder = shared_http_client()
        .get(&url)
        .timeout(GATEWAY_GET_TIMEOUT);
    execute_gateway_json(builder, gateway, GatewayRetryPolicy::Idempotent).await
}

pub(super) fn print_pretty_json(value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

pub(super) async fn post_gateway_json(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let builder = shared_http_client()
        .post(&url)
        .json(payload)
        .timeout(GATEWAY_MUTATION_TIMEOUT);
    execute_gateway_json(builder, gateway, GatewayRetryPolicy::ConnectOnly).await
}

pub(super) async fn post_gateway_json_as_cli_actor(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let builder = shared_http_client()
        .post(&url)
        .header("X-Garyx-Actor", cli_actor_header_value())
        .json(payload)
        .timeout(GATEWAY_MUTATION_TIMEOUT);
    execute_gateway_json(builder, gateway, GatewayRetryPolicy::ConnectOnly).await
}

pub(super) async fn patch_gateway_json(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let builder = shared_http_client()
        .patch(&url)
        .json(payload)
        .timeout(GATEWAY_MUTATION_TIMEOUT);
    execute_gateway_json(builder, gateway, GatewayRetryPolicy::ConnectOnly).await
}

pub(super) async fn patch_gateway_json_as_cli_actor(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let builder = shared_http_client()
        .patch(&url)
        .header("X-Garyx-Actor", cli_actor_header_value())
        .json(payload)
        .timeout(GATEWAY_MUTATION_TIMEOUT);
    execute_gateway_json(builder, gateway, GatewayRetryPolicy::ConnectOnly).await
}

pub(super) async fn put_gateway_json(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let builder = shared_http_client()
        .put(&url)
        .json(payload)
        .timeout(GATEWAY_MUTATION_TIMEOUT);
    execute_gateway_json(builder, gateway, GatewayRetryPolicy::ConnectOnly).await
}

pub(super) async fn delete_gateway_json_as_cli_actor(
    gateway: &GatewayEndpoint,
    path: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let builder = shared_http_client()
        .delete(&url)
        .header("X-Garyx-Actor", cli_actor_header_value())
        .timeout(GATEWAY_MUTATION_TIMEOUT);
    execute_gateway_json(builder, gateway, GatewayRetryPolicy::ConnectOnly).await
}

pub(super) async fn delete_gateway_json(
    gateway: &GatewayEndpoint,
    path: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let builder = shared_http_client()
        .delete(&url)
        .timeout(GATEWAY_MUTATION_TIMEOUT);
    execute_gateway_json(builder, gateway, GatewayRetryPolicy::ConnectOnly).await
}

pub(super) fn principal_payload(principal: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let principal = principal.trim();
    if principal.is_empty() {
        return Err("principal cannot be empty".into());
    }
    if let Some(user_id) = principal.strip_prefix("human:") {
        let user_id = user_id.trim();
        if user_id.is_empty() {
            return Err("human principal cannot be empty".into());
        }
        return Ok(json!({ "kind": "human", "user_id": user_id }));
    }
    if let Some(agent_id) = principal.strip_prefix("agent:") {
        let agent_id = agent_id.trim();
        if agent_id.is_empty() {
            return Err("agent principal cannot be empty".into());
        }
        return Ok(json!({ "kind": "agent", "agent_id": agent_id }));
    }
    Ok(json!({ "kind": "agent", "agent_id": principal }))
}

fn cli_actor_payload() -> Value {
    if let Some(actor) = env_nonempty("GARYX_ACTOR") {
        return principal_payload(&actor)
            .unwrap_or_else(|_| json!({ "kind": "human", "user_id": cli_actor_user_id() }));
    }
    if let Some(agent_id) = env_nonempty("GARYX_AGENT_ID") {
        return json!({ "kind": "agent", "agent_id": agent_id });
    }
    json!({ "kind": "human", "user_id": cli_actor_user_id() })
}

fn cli_actor_user_id() -> String {
    env_nonempty("GARYX_USER").unwrap_or_else(|| "owner".to_owned())
}

fn cli_actor_header_value() -> String {
    format_principal(&cli_actor_payload())
}

pub(super) fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub(super) fn format_principal(value: &Value) -> String {
    if value.is_null() {
        return "(unassigned)".to_owned();
    }
    match value
        .get("kind")
        .or_else(|| value.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("-")
    {
        "human" => format!(
            "human:{}",
            value.get("user_id").and_then(Value::as_str).unwrap_or("-")
        ),
        "agent" => format!(
            "agent:{}",
            value.get("agent_id").and_then(Value::as_str).unwrap_or("-")
        ),
        other => other.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn retry_table_allows_connect_retries_for_both_policies() {
        for policy in [
            GatewayRetryPolicy::Idempotent,
            GatewayRetryPolicy::ConnectOnly,
        ] {
            assert!(transport_failure_retriable(policy, true, false, 1, 0));
            assert!(transport_failure_retriable(policy, true, false, 2, 0));
            assert!(
                !transport_failure_retriable(policy, true, false, 3, 0),
                "third connect failure must exhaust for {policy:?}"
            );
        }
    }

    #[test]
    fn retry_table_allows_one_timeout_retry_for_idempotent_only() {
        assert!(transport_failure_retriable(
            GatewayRetryPolicy::Idempotent,
            false,
            true,
            1,
            1
        ));
        assert!(!transport_failure_retriable(
            GatewayRetryPolicy::Idempotent,
            false,
            true,
            2,
            2
        ));
        assert!(!transport_failure_retriable(
            GatewayRetryPolicy::ConnectOnly,
            false,
            true,
            1,
            1
        ));
    }

    #[test]
    fn retry_table_never_retries_other_transport_failures() {
        assert!(!transport_failure_retriable(
            GatewayRetryPolicy::Idempotent,
            false,
            false,
            1,
            0
        ));
        assert!(!transport_failure_retriable(
            GatewayRetryPolicy::ConnectOnly,
            false,
            false,
            1,
            0
        ));
    }

    #[test]
    fn retry_backoff_steps_then_saturates() {
        assert_eq!(retry_backoff(1), Duration::from_millis(250));
        assert_eq!(retry_backoff(2), Duration::from_millis(750));
        assert_eq!(retry_backoff(9), Duration::from_millis(750));
    }

    /// Behavior for each accepted connection, in order; later connections
    /// reuse the last entry.
    #[derive(Clone, Copy)]
    enum TestConn {
        /// Hold the connection open without responding for this long.
        StallMs(u64),
        /// Read the request and answer `200 {"ok":true}`.
        Ok,
    }

    async fn spawn_test_server(behaviors: Vec<TestConn>) -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let connections = Arc::new(AtomicUsize::new(0));
        let seen = connections.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let index = seen.fetch_add(1, Ordering::SeqCst);
                let behavior = *behaviors
                    .get(index)
                    .or_else(|| behaviors.last())
                    .expect("behavior list non-empty");
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = stream.read(&mut buf).await;
                    match behavior {
                        TestConn::StallMs(ms) => {
                            tokio::time::sleep(Duration::from_millis(ms)).await;
                        }
                        TestConn::Ok => {
                            let body = "{\"ok\":true}";
                            let response = format!(
                                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                        }
                    }
                });
            }
        });
        (format!("http://{addr}"), connections)
    }

    fn test_endpoint(base_url: String) -> GatewayEndpoint {
        GatewayEndpoint {
            base_url,
            auth_token: None,
        }
    }

    #[tokio::test]
    async fn idempotent_request_retries_a_timeout_and_succeeds() {
        let (base_url, connections) =
            spawn_test_server(vec![TestConn::StallMs(2_000), TestConn::Ok]).await;
        let gateway = test_endpoint(base_url);
        let builder = shared_http_client()
            .get(format!("{}/api/tasks", gateway.base_url))
            .timeout(Duration::from_millis(150));

        let value = execute_gateway_json(builder, &gateway, GatewayRetryPolicy::Idempotent)
            .await
            .expect("retry succeeds");

        assert_eq!(value, json!({ "ok": true }));
        assert_eq!(connections.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn idempotent_request_stops_after_two_timeout_attempts() {
        let (base_url, connections) = spawn_test_server(vec![TestConn::StallMs(2_000)]).await;
        let gateway = test_endpoint(base_url);
        let builder = shared_http_client()
            .get(format!("{}/api/tasks", gateway.base_url))
            .timeout(Duration::from_millis(150));

        let error = execute_gateway_json(builder, &gateway, GatewayRetryPolicy::Idempotent)
            .await
            .expect_err("stalled server fails");

        let cli_error = error
            .downcast_ref::<GatewayCliError>()
            .expect("gateway cli error");
        assert_eq!(cli_error.kind, GatewayErrorKind::Unreachable);
        assert!(cli_error.message.contains("did not respond in time"));
        assert_eq!(connections.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn mutation_does_not_retry_a_timeout() {
        let (base_url, connections) = spawn_test_server(vec![TestConn::StallMs(2_000)]).await;
        let gateway = test_endpoint(base_url);
        let builder = shared_http_client()
            .post(format!("{}/api/tasks", gateway.base_url))
            .json(&json!({ "title": "t" }))
            .timeout(Duration::from_millis(150));

        let error = execute_gateway_json(builder, &gateway, GatewayRetryPolicy::ConnectOnly)
            .await
            .expect_err("stalled server fails");

        let cli_error = error
            .downcast_ref::<GatewayCliError>()
            .expect("gateway cli error");
        assert_eq!(cli_error.kind, GatewayErrorKind::Unreachable);
        assert!(
            cli_error.message.contains("verify whether the write landed"),
            "mutation timeout should warn before manual retry: {}",
            cli_error.message
        );
        assert_eq!(connections.load(Ordering::SeqCst), 1);
    }

    /// Bind a port, drop the listener (subsequent connects are refused —
    /// the gateway restart window), then bring a real listener back on the
    /// same port after `revive_after_ms`.
    async fn spawn_refused_then_ok_server(revive_after_ms: u64) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind probe listener");
        let addr = listener.local_addr().expect("listener addr");
        drop(listener);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(revive_after_ms)).await;
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .expect("rebind revived listener");
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = stream.read(&mut buf).await;
                    let body = "{\"ok\":true}";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn idempotent_request_rides_out_a_restart_window() {
        // Connection refused (no listener), gateway comes back before the
        // retry budget runs out: the GET must succeed via retry.
        let base_url = spawn_refused_then_ok_server(100).await;
        let gateway = test_endpoint(base_url);
        let builder = shared_http_client()
            .get(format!("{}/api/tasks", gateway.base_url))
            .timeout(GATEWAY_GET_TIMEOUT);

        let value = execute_gateway_json(builder, &gateway, GatewayRetryPolicy::Idempotent)
            .await
            .expect("retry rides out the restart window");

        assert_eq!(value, json!({ "ok": true }));
    }

    #[tokio::test]
    async fn mutation_rides_out_a_restart_window() {
        // A refused connection never reached a listener, so retrying a
        // mutation is safe — this is the restart-window case the retry
        // exists for.
        let base_url = spawn_refused_then_ok_server(100).await;
        let gateway = test_endpoint(base_url);
        let builder = shared_http_client()
            .post(format!("{}/api/tasks", gateway.base_url))
            .json(&json!({ "title": "t" }))
            .timeout(GATEWAY_MUTATION_TIMEOUT);

        let value = execute_gateway_json(builder, &gateway, GatewayRetryPolicy::ConnectOnly)
            .await
            .expect("mutation retries a refused connection");

        assert_eq!(value, json!({ "ok": true }));
    }

    #[tokio::test]
    async fn persistent_refusal_exhausts_connect_attempts() {
        // Gateway never comes back: three connect attempts, then the
        // familiar "not reachable" failure.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind probe listener");
        let addr = listener.local_addr().expect("listener addr");
        drop(listener);
        let gateway = test_endpoint(format!("http://{addr}"));

        let error = fetch_gateway_json(&gateway, "/api/tasks")
            .await
            .expect_err("dead gateway fails after retries");

        let cli_error = error
            .downcast_ref::<GatewayCliError>()
            .expect("gateway cli error");
        assert_eq!(cli_error.kind, GatewayErrorKind::Unreachable);
        assert!(
            cli_error.message.contains("not reachable"),
            "refused connections keep the reachability text: {}",
            cli_error.message
        );
    }

    #[tokio::test]
    async fn non_success_status_is_never_retried() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let connections = Arc::new(AtomicUsize::new(0));
        let seen = connections.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                seen.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf).await;
                let body = "{\"error\":\"nope\"}";
                let response = format!(
                    "HTTP/1.1 500 Internal Server Error\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });
        let gateway = test_endpoint(format!("http://{addr}"));

        let error = fetch_gateway_json(&gateway, "/api/tasks")
            .await
            .expect_err("500 fails");

        let cli_error = error
            .downcast_ref::<GatewayCliError>()
            .expect("gateway cli error");
        assert_eq!(cli_error.kind, GatewayErrorKind::Rejected);
        assert_eq!(connections.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cli_actor_header_uses_agent_identity_from_env() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let _actor = ScopedEnvVar::remove("GARYX_ACTOR");
        let _agent = ScopedEnvVar::set_string("GARYX_AGENT_ID", "codex");
        let _user = ScopedEnvVar::set_string("GARYX_USER", "owner");

        assert_eq!(cli_actor_header_value(), "agent:codex");
        assert_eq!(
            cli_actor_payload(),
            json!({ "kind": "agent", "agent_id": "codex" })
        );
    }

    #[test]
    fn cli_actor_header_prefers_explicit_actor_env() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let _actor = ScopedEnvVar::set_string("GARYX_ACTOR", "human:alice");
        let _agent = ScopedEnvVar::set_string("GARYX_AGENT_ID", "codex");
        let _user = ScopedEnvVar::set_string("GARYX_USER", "owner");

        assert_eq!(cli_actor_header_value(), "human:alice");
        assert_eq!(
            cli_actor_payload(),
            json!({ "kind": "human", "user_id": "alice" })
        );
    }

    #[test]
    fn gateway_base_url_prefers_public_url() {
        let dir = tempdir().expect("tempdir");
        let config_path = dir.path().join("gary.json");
        std::fs::write(
            &config_path,
            serde_json::to_vec_pretty(&json!({
                "gateway": {
                    "host": "0.0.0.0",
                    "port": 3000,
                    "public_url": "http://127.0.0.1:31337"
                }
            }))
            .expect("config json"),
        )
        .expect("write config");
        let base_url =
            gateway_base_url(config_path.to_str().expect("config path")).expect("base url");
        assert_eq!(base_url, "http://127.0.0.1:31337");
    }
}
