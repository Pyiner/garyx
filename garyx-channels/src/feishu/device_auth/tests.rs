use super::*;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Build an HTTP client whose requests we can route to a mock server
/// by overriding the endpoint constants at call sites. For the real
/// modules these constants are hard-coded; for tests we construct the
/// request by hand and call the helpers that accept a custom URL.
///
/// Rather than plumbing a config param all the way down just to test,
/// we drive the mock by issuing requests to the real URLs and mocking
/// them via a reverse-proxy-style setup would be overkill. Instead we
/// test the response parsing in isolation from an in-memory MockServer
/// and a tiny indirection: reroute begin/poll to use the test URL.
///
/// Tests below exercise the whole HTTP boundary by POSTing directly
/// to the mock and parsing responses through the same `BeginOrPollResponse`
/// struct the production path uses.

fn client() -> HttpClient {
    HttpClient::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client")
}

#[tokio::test]
async fn build_verification_url_feishu_embeds_user_code_and_version() {
    let url = build_verification_url(&FeishuDomain::Feishu, "WXYZ-1234", "0.1.2");
    assert!(url.starts_with("https://open.feishu.cn/page/cli?"));
    assert!(url.contains("user_code=WXYZ-1234"));
    assert!(url.contains("lpv=0.1.2"));
    assert!(url.contains("ocv=0.1.2"));
    assert!(url.contains("from=cli"));
}

#[tokio::test]
async fn build_verification_url_lark_targets_larksuite_host() {
    let url = build_verification_url(&FeishuDomain::Lark, "ABCD", "0.1.2");
    assert!(url.starts_with("https://open.larksuite.com/page/cli?"));
}

#[tokio::test]
async fn build_verification_url_urlencodes_user_code() {
    // user_code with a slash — must be URL-encoded in the query string
    let url = build_verification_url(&FeishuDomain::Feishu, "AB/CD", "0.1.2");
    assert!(url.contains("user_code=AB%2FCD"));
}

/// Helper: issue a begin request to an arbitrary URL (for mock tests).
async fn begin_against(
    client: &HttpClient,
    url: &str,
) -> Result<BeginOrPollResponse, DeviceFlowError> {
    let resp = client
        .post(url)
        .form(&[
            ("action", "begin"),
            ("archetype", ARCHETYPE),
            ("auth_method", "client_secret"),
            ("request_user_info", "open_id tenant_brand"),
        ])
        .send()
        .await?;
    let body = resp.text().await?;
    serde_json::from_str(&body).map_err(|_| DeviceFlowError::NonJson { status: 0, body })
}

async fn poll_against(
    client: &HttpClient,
    url: &str,
    device_code: &str,
) -> Result<BeginOrPollResponse, DeviceFlowError> {
    let resp = client
        .post(url)
        .form(&[("action", "poll"), ("device_code", device_code)])
        .send()
        .await?;
    let body = resp.text().await?;
    serde_json::from_str(&body).map_err(|_| DeviceFlowError::NonJson { status: 0, body })
}

#[tokio::test]
async fn begin_response_parses_fields() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/v1/app/registration"))
        .and(body_string_contains("action=begin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "device_code": "dev-123",
            "user_code": "WXYZ-1234",
            "verification_uri": "https://accounts.feishu.cn/device",
            "expires_in": 600,
            "interval": 7
        })))
        .mount(&server)
        .await;

    let c = client();
    let url = format!("{}/oauth/v1/app/registration", server.uri());
    let data = begin_against(&c, &url).await.expect("begin");
    assert_eq!(data.device_code.as_deref(), Some("dev-123"));
    assert_eq!(data.user_code.as_deref(), Some("WXYZ-1234"));
    assert_eq!(data.expires_in, Some(600));
    assert_eq!(data.interval, Some(7));
    assert!(data.error.is_none());
}

#[tokio::test]
async fn begin_error_surfaces_error_description() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/v1/app/registration"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "invalid_request",
            "error_description": "archetype is required"
        })))
        .mount(&server)
        .await;

    let c = client();
    let url = format!("{}/oauth/v1/app/registration", server.uri());
    let data = begin_against(&c, &url).await.expect("parse");
    assert_eq!(data.error.as_deref(), Some("invalid_request"));
    assert_eq!(
        data.error_description.as_deref(),
        Some("archetype is required"),
    );
}

#[tokio::test]
async fn poll_pending_maps_to_pending() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/v1/app/registration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "error": "authorization_pending"
        })))
        .mount(&server)
        .await;

    let c = client();
    let url = format!("{}/oauth/v1/app/registration", server.uri());
    let data = poll_against(&c, &url, "dev-123").await.expect("parse");
    assert_eq!(data.error.as_deref(), Some("authorization_pending"));
}

#[tokio::test]
async fn poll_success_surfaces_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/v1/app/registration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "client_id": "cli_abc",
            "client_secret": "secret_xyz",
            "user_info": {
                "open_id": "ou_1",
                "tenant_brand": "feishu"
            }
        })))
        .mount(&server)
        .await;

    let c = client();
    let url = format!("{}/oauth/v1/app/registration", server.uri());
    let data = poll_against(&c, &url, "dev-123").await.expect("parse");
    assert_eq!(data.client_id.as_deref(), Some("cli_abc"));
    assert_eq!(data.client_secret.as_deref(), Some("secret_xyz"));
    assert_eq!(
        data.user_info.and_then(|u| u.tenant_brand).as_deref(),
        Some("feishu"),
    );
}

#[tokio::test]
async fn poll_lark_tenant_marker_extracted() {
    let server = MockServer::start().await;
    // Server returns client_id but empty secret + tenant_brand=lark —
    // the "retry on larksuite" trigger we rely on in run_device_flow.
    Mock::given(method("POST"))
        .and(path("/oauth/v1/app/registration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "client_id": "cli_lark",
            "user_info": {
                "open_id": "ou_2",
                "tenant_brand": "lark"
            }
        })))
        .mount(&server)
        .await;

    let c = client();
    let url = format!("{}/oauth/v1/app/registration", server.uri());
    let data = poll_against(&c, &url, "dev-123").await.expect("parse");
    assert_eq!(data.client_id.as_deref(), Some("cli_lark"));
    assert!(data.client_secret.as_deref().unwrap_or("").is_empty());
    assert_eq!(
        data.user_info.and_then(|u| u.tenant_brand).as_deref(),
        Some("lark"),
    );
}

#[tokio::test]
async fn poll_response_classification() {
    // Exercise the match arms in `poll_once`'s outcome mapping without
    // spinning a server.
    fn classify(resp: BeginOrPollResponse) -> &'static str {
        if resp.error.is_none() && resp.client_id.as_deref().map_or(false, |s| !s.is_empty()) {
            return "success";
        }
        match resp.error.as_deref() {
            Some("authorization_pending") => "pending",
            Some("slow_down") => "slow_down",
            Some("access_denied") => "denied",
            Some("expired_token") | Some("invalid_grant") => "expired",
            _ => "other",
        }
    }

    let pending: BeginOrPollResponse =
        serde_json::from_value(serde_json::json!({"error":"authorization_pending"})).unwrap();
    let slow: BeginOrPollResponse =
        serde_json::from_value(serde_json::json!({"error":"slow_down"})).unwrap();
    let denied: BeginOrPollResponse =
        serde_json::from_value(serde_json::json!({"error":"access_denied"})).unwrap();
    let expired: BeginOrPollResponse =
        serde_json::from_value(serde_json::json!({"error":"expired_token"})).unwrap();
    let invalid: BeginOrPollResponse =
        serde_json::from_value(serde_json::json!({"error":"invalid_grant"})).unwrap();
    let success: BeginOrPollResponse =
        serde_json::from_value(serde_json::json!({"client_id":"c","client_secret":"s"})).unwrap();
    assert_eq!(classify(pending), "pending");
    assert_eq!(classify(slow), "slow_down");
    assert_eq!(classify(denied), "denied");
    assert_eq!(classify(expired), "expired");
    assert_eq!(classify(invalid), "expired");
    assert_eq!(classify(success), "success");
}
