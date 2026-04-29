//! Differential test for `WeixinAuthExecutor` against the pure
//! `begin_qr_login_at` + `poll_qr_login_status_at` primitives.
//!
//! Weixin's state machine has a `scanned` intermediate the other
//! channels don't — this test pins that it shows up as a
//! `Pending` with a display refresh, not a terminal state, and
//! that the final `{bot_token, base_url, ilink_bot_id}` values
//! round-trip identically through both paths.
//!
//! Contract: the same wiremock script must produce identical
//! terminal values AND the same request sequence on both paths.

use std::time::Duration;

use garyx_channels::WeixinAuthExecutor;
use garyx_channels::auth_flow::{AuthDisplayItem, AuthFlowExecutor, AuthPollResult};
use garyx_channels::weixin_auth::{WeixinPollStatus, begin_qr_login_at, poll_qr_login_status_at};
use reqwest::Client;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const BEGIN_PATH: &str = "/ilink/bot/get_bot_qrcode";
const POLL_PATH: &str = "/ilink/bot/get_qrcode_status";
const QRCODE: &str = "wx-nonce-42";
const QRCODE_IMAGE: &str = "PNG_BASE64_BODY";
const BOT_ID: &str = "bot-id-42";
const BOT_TOKEN: &str = "bot-token-42";
const BASEURL: &str = "https://weixin.example";

async fn install_script(server: &MockServer) {
    // 1. begin
    Mock::given(method("GET"))
        .and(path(BEGIN_PATH))
        .and(query_param("bot_type", "3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "qrcode": QRCODE,
            "qrcode_img_content": QRCODE_IMAGE,
        })))
        .mount(server)
        .await;

    // 2. 2 wait
    Mock::given(method("GET"))
        .and(path(POLL_PATH))
        .and(query_param("qrcode", QRCODE))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "wait" })))
        .up_to_n_times(2)
        .mount(server)
        .await;

    // 3. 1 scaned (weixin's misspelling of "scanned")
    Mock::given(method("GET"))
        .and(path(POLL_PATH))
        .and(query_param("qrcode", QRCODE))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "scaned" })))
        .up_to_n_times(1)
        .mount(server)
        .await;

    // 4. confirmed
    Mock::given(method("GET"))
        .and(path(POLL_PATH))
        .and(query_param("qrcode", QRCODE))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "confirmed",
            "bot_token": BOT_TOKEN,
            "ilink_bot_id": BOT_ID,
            "baseurl": BASEURL,
        })))
        .mount(server)
        .await;
}

fn http() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("test client")
}

async fn run_old(server: &MockServer) -> (String, String, String, usize) {
    let http = http();
    let begin_url = format!("{}{}", server.uri(), BEGIN_PATH);
    let poll_url = format!("{}{}", server.uri(), POLL_PATH);
    let begin = begin_qr_login_at(&http, &begin_url).await.expect("begin");
    assert_eq!(begin.qrcode, QRCODE);
    let mut requests = 1;
    for _ in 0..20 {
        match poll_qr_login_status_at(&http, &poll_url, &begin.qrcode, &server.uri())
            .await
            .expect("poll")
        {
            WeixinPollStatus::Pending => {
                requests += 1;
            }
            WeixinPollStatus::Scanned => {
                requests += 1;
            }
            WeixinPollStatus::Confirmed(c) => {
                requests += 1;
                return (c.bot_token, c.base_url, c.ilink_bot_id, requests);
            }
        }
    }
    panic!("old path never terminated");
}

async fn run_new(server: &MockServer) -> (String, String, String, usize) {
    let begin_url = format!("{}{}", server.uri(), BEGIN_PATH);
    let poll_url = format!("{}{}", server.uri(), POLL_PATH);
    let executor = WeixinAuthExecutor::with_endpoint_override(http(), begin_url, poll_url);
    let session = executor
        .start(json!({ "base_url": server.uri() }))
        .await
        .expect("start");

    // Weixin's initial display is a hint + the server-provided QR payload.
    // The opaque nonce remains internal and is used only for polling.
    assert!(
        session.display.iter().any(|item| matches!(
            item,
            AuthDisplayItem::Qr { value } if value == QRCODE_IMAGE,
        )),
        "weixin initial display must include the QR payload",
    );
    assert!(
        session
            .display
            .iter()
            .any(|item| matches!(item, AuthDisplayItem::Text { .. })),
        "weixin initial display must include a hint",
    );

    let mut requests = 1;
    let mut saw_scanned_refresh = false;
    for _ in 0..20 {
        match executor.poll(&session.session_id).await.expect("poll") {
            AuthPollResult::Pending { display, .. } => {
                requests += 1;
                // The "scaned" tick must surface as a Pending with
                // a replacement display (no QR, just the status
                // update). We don't assert it on every Pending —
                // only one of them, which comes from the server's
                // "scaned" response.
                if let Some(items) = display {
                    let has_qr = items
                        .iter()
                        .any(|i| matches!(i, AuthDisplayItem::Qr { .. }));
                    let has_confirm_hint = items.iter().any(
                        |i| matches!(i, AuthDisplayItem::Text { value } if value.contains("确认")),
                    );
                    assert!(!has_qr, "scanned-state refresh must not re-render QR");
                    assert!(
                        has_confirm_hint,
                        "scanned-state refresh must prompt user to confirm",
                    );
                    saw_scanned_refresh = true;
                }
            }
            AuthPollResult::Confirmed { values } => {
                requests += 1;
                assert!(
                    saw_scanned_refresh,
                    "executor must surface a scanned-state display refresh before Confirmed",
                );
                let token = values["token"].as_str().unwrap().to_owned();
                let base_url = values["base_url"].as_str().unwrap().to_owned();
                let account_id = values["account_id"].as_str().unwrap().to_owned();
                return (token, base_url, account_id, requests);
            }
            AuthPollResult::Failed { reason } => {
                panic!("unexpected Failed: {reason}");
            }
        }
    }
    panic!("new path never terminated");
}

#[tokio::test]
async fn weixin_executor_matches_raw_driver_happy_path() {
    let server_old = MockServer::start().await;
    install_script(&server_old).await;
    let server_new = MockServer::start().await;
    install_script(&server_new).await;

    let (old_token, old_base, old_id, old_reqs) = run_old(&server_old).await;
    let (new_token, new_base, new_id, new_reqs) = run_new(&server_new).await;

    assert_eq!(old_token, new_token, "bot_token differs");
    assert_eq!(old_base, new_base, "base_url differs");
    assert_eq!(old_id, new_id, "ilink_bot_id differs");
    assert_eq!(old_reqs, new_reqs, "HTTP request count differs");

    let old_requests = server_old.received_requests().await.unwrap();
    let new_requests = server_new.received_requests().await.unwrap();
    assert_eq!(old_requests.len(), new_requests.len());
    for (i, (o, n)) in old_requests.iter().zip(new_requests.iter()).enumerate() {
        assert_eq!(
            o.method.to_string(),
            n.method.to_string(),
            "req #{i} method"
        );
        assert_eq!(o.url.path(), n.url.path(), "req #{i} path");
        assert_eq!(o.url.query(), n.url.query(), "req #{i} query");
    }
}
