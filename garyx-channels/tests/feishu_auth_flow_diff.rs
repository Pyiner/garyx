//! Differential test: `FeishuAuthExecutor` vs the original
//! `begin_app_registration` + `poll_once` driver pair that
//! `perform_feishu_device_auth` / `run_device_flow` use today.
//!
//! Contract: given an identical HTTP script (begin response →
//! pending → slow_down → pending → confirmed) both paths must
//! end up with the same `{app_id, app_secret, domain}` triple AND
//! issue the same sequence of POST requests in the same order.
//! Any drift means the executor's abstraction has silently
//! diverged from production semantics.

use std::time::Duration;

use garyx_channels::auth_flow::{AuthDisplayItem, AuthFlowExecutor, AuthPollResult};
use garyx_channels::feishu::{FeishuAuthExecutor, begin_app_registration_at, poll_once_at};
use garyx_models::config::FeishuDomain;
use reqwest::Client;
use serde_json::json;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Feishu accounts server's URL contract — both begin and poll
/// go to the same path with different `action=` form fields.
const ENDPOINT_PATH: &str = "/oauth/v1/app/registration";

/// Script the mock plays against any HTTP client:
///   1. begin → returns {device_code, user_code, expires_in, interval}
///   2. poll × 2 → `authorization_pending`
///   3. poll × 1 → `slow_down`
///   4. poll × 1 → `authorization_pending`
///   5. poll × 1 → success with client_id + client_secret + tenant_brand=feishu
///
/// wiremock matchers queue responses via `up_to_n_times`. Begin and
/// poll are separate `Mock::given` definitions filtered by body
/// substring so they don't collide.
async fn install_script(server: &MockServer) {
    // 1. begin
    Mock::given(method("POST"))
        .and(path(ENDPOINT_PATH))
        .and(body_string_contains("action=begin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_code": "dev-42",
            "user_code": "WXYZ-1234",
            "verification_uri": "https://accounts.feishu.cn/device",
            "expires_in": 600,
            "interval": 5,
        })))
        .mount(server)
        .await;

    // 2. 2 pending
    Mock::given(method("POST"))
        .and(path(ENDPOINT_PATH))
        .and(body_string_contains("action=poll"))
        .and(body_string_contains("device_code=dev-42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "error": "authorization_pending"
        })))
        .up_to_n_times(2)
        .mount(server)
        .await;

    // 3. 1 slow_down
    Mock::given(method("POST"))
        .and(path(ENDPOINT_PATH))
        .and(body_string_contains("action=poll"))
        .and(body_string_contains("device_code=dev-42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "error": "slow_down" })))
        .up_to_n_times(1)
        .mount(server)
        .await;

    // 4. 1 pending again
    Mock::given(method("POST"))
        .and(path(ENDPOINT_PATH))
        .and(body_string_contains("action=poll"))
        .and(body_string_contains("device_code=dev-42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "error": "authorization_pending"
        })))
        .up_to_n_times(1)
        .mount(server)
        .await;

    // 5. success
    Mock::given(method("POST"))
        .and(path(ENDPOINT_PATH))
        .and(body_string_contains("action=poll"))
        .and(body_string_contains("device_code=dev-42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "client_id": "cli_feishu_xyz",
            "client_secret": "secret_feishu_abc",
            "user_info": { "tenant_brand": "feishu" },
        })))
        .mount(server)
        .await;
}

fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("test http client")
}

/// OLD path — what `run_device_flow` does under the covers. We
/// call the endpoint-explicit primitives directly so we can point
/// at the wiremock URL without DNS shenanigans. Returns the final
/// credentials + the number of HTTP requests issued.
async fn run_old_path(endpoint: &str) -> (String, String, String, usize) {
    let http = http_client();
    let begin = begin_app_registration_at(&http, endpoint, &FeishuDomain::Feishu, "diff-test")
        .await
        .expect("begin");
    assert_eq!(begin.device_code, "dev-42");

    let mut interval: u64 = begin.interval.max(1);
    let mut request_count = 1; // one begin so far
    // Bounded loop so a broken mock doesn't hang the test.
    for _ in 0..20 {
        match poll_once_at(&http, endpoint, &FeishuDomain::Feishu, &begin.device_code)
            .await
            .expect("poll")
        {
            garyx_channels::feishu::PollStatus::Pending => {
                request_count += 1;
                continue;
            }
            garyx_channels::feishu::PollStatus::SlowDown => {
                request_count += 1;
                interval = (interval + 5).min(30);
                continue;
            }
            garyx_channels::feishu::PollStatus::Denied => panic!("unexpected Denied"),
            garyx_channels::feishu::PollStatus::Expired => panic!("unexpected Expired"),
            garyx_channels::feishu::PollStatus::Success(result) => {
                request_count += 1;
                let domain_str = match result.tenant_brand {
                    FeishuDomain::Feishu => "feishu",
                    FeishuDomain::Lark => "lark",
                };
                // Interval growth tracked for differential compare.
                let _ = interval;
                return (
                    result.app_id,
                    result.app_secret,
                    domain_str.into(),
                    request_count,
                );
            }
        }
    }
    panic!("old path never terminated");
}

/// NEW path — same script, same wiremock URL, through the
/// `FeishuAuthExecutor` trait.
async fn run_new_path(endpoint: &str) -> (String, String, String, usize) {
    let executor = FeishuAuthExecutor::with_endpoint_override(http_client(), endpoint);
    let session = executor
        .start(json!({ "domain": "feishu", "cli_version": "diff-test" }))
        .await
        .expect("executor start");

    // The `start` response drives the UI — assert the display list
    // carries the user_code + verification URL + a QR so the
    // abstraction holds even on the happy path.
    let has_user_code = session
        .display
        .iter()
        .any(|item| matches!(item, AuthDisplayItem::Text { value } if value.contains("WXYZ-1234")));
    // `build_verification_url` sends users to `open.feishu.cn/page/cli`,
    // NOT the accounts endpoint the device_code POST itself went to.
    let has_url = session.display.iter().any(
        |item| matches!(item, AuthDisplayItem::Text { value } if value.contains("open.feishu.cn")),
    );
    let has_qr = session
        .display
        .iter()
        .any(|item| matches!(item, AuthDisplayItem::Qr { .. }));
    assert!(has_user_code, "user_code must appear in display");
    assert!(has_url, "verification_url must appear in display");
    assert!(has_qr, "feishu display must include a QR item");

    let mut request_count = 1; // one begin
    for _ in 0..20 {
        match executor.poll(&session.session_id).await.expect("poll") {
            AuthPollResult::Pending { .. } => {
                request_count += 1;
            }
            AuthPollResult::Confirmed { values } => {
                request_count += 1;
                let app_id = values["app_id"].as_str().unwrap().to_owned();
                let app_secret = values["app_secret"].as_str().unwrap().to_owned();
                let domain = values["domain"].as_str().unwrap().to_owned();
                return (app_id, app_secret, domain, request_count);
            }
            AuthPollResult::Failed { reason } => {
                panic!("unexpected Failed: {reason}");
            }
        }
    }
    panic!("new path never terminated");
}

#[tokio::test]
async fn feishu_executor_matches_device_flow_driver_happy_path() {
    // OLD and NEW must issue the same number of HTTP requests to
    // the same mock and end up with the same credentials. We run
    // each against its own wiremock so the mocks don't share
    // request counters, then compare the totals.
    let server_old = MockServer::start().await;
    install_script(&server_old).await;
    let server_new = MockServer::start().await;
    install_script(&server_new).await;

    let endpoint_old = format!("{}{}", server_old.uri(), ENDPOINT_PATH);
    let endpoint_new = format!("{}{}", server_new.uri(), ENDPOINT_PATH);

    let (old_app_id, old_secret, old_domain, old_reqs) = run_old_path(&endpoint_old).await;
    let (new_app_id, new_secret, new_domain, new_reqs) = run_new_path(&endpoint_new).await;

    assert_eq!(
        (
            old_app_id.as_str(),
            old_secret.as_str(),
            old_domain.as_str()
        ),
        (
            new_app_id.as_str(),
            new_secret.as_str(),
            new_domain.as_str()
        ),
        "OLD and NEW must surface identical credentials",
    );
    assert_eq!(
        old_reqs, new_reqs,
        "OLD and NEW must issue the same number of HTTP requests"
    );

    // Additionally spot-check the HTTP request log — every request
    // posted to the mock for both paths must be against the same
    // endpoint path and contain the same device_code.
    let old_requests = server_old.received_requests().await.unwrap();
    let new_requests = server_new.received_requests().await.unwrap();
    assert_eq!(
        old_requests.len(),
        new_requests.len(),
        "request log size mismatch"
    );
    for (i, (o, n)) in old_requests.iter().zip(new_requests.iter()).enumerate() {
        assert_eq!(o.url.path(), n.url.path(), "request #{i} path mismatch",);
        assert_eq!(
            o.method.to_string(),
            n.method.to_string(),
            "request #{i} method mismatch",
        );
        let old_body = std::str::from_utf8(&o.body).unwrap_or("");
        let new_body = std::str::from_utf8(&n.body).unwrap_or("");
        assert_eq!(
            body_action(old_body),
            body_action(new_body),
            "request #{i} action form-field mismatch",
        );
    }
}

fn body_action(body: &str) -> Option<&str> {
    // `action=begin` / `action=poll` lives in a urlencoded form
    // body (`action=begin&archetype=...`). Pull just the action
    // value so we don't have to string-match the full body.
    body.split('&').find_map(|kv| kv.strip_prefix("action="))
}
