use super::*;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn begin_returns_qrcode() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ilink/bot/get_bot_qrcode"))
        .and(query_param("bot_type", "3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "qrcode": "qr-token-1",
            "qrcode_img_content": "iVBORw0KGgo=",
        })))
        .mount(&server)
        .await;
    let r = begin_qr_login(&HttpClient::new(), &server.uri())
        .await
        .unwrap();
    assert_eq!(r.qrcode, "qr-token-1");
    assert_eq!(r.qrcode_img_content, "iVBORw0KGgo=");
}

#[tokio::test]
async fn poll_maps_three_states_and_header_is_sent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ilink/bot/get_qrcode_status"))
        .and(query_param("qrcode", "qr-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "wait" })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/ilink/bot/get_qrcode_status"))
        .and(query_param("qrcode", "qr-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "scaned" })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/ilink/bot/get_qrcode_status"))
        .and(query_param("qrcode", "qr-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "confirmed",
            "bot_token": "tok-final",
            "ilink_bot_id": "bot-123",
            "baseurl": "https://cluster.example",
        })))
        .mount(&server)
        .await;

    let client = HttpClient::new();
    assert!(matches!(
        poll_qr_login_status(&client, &server.uri(), "qr-1")
            .await
            .unwrap(),
        WeixinPollStatus::Pending
    ));
    assert!(matches!(
        poll_qr_login_status(&client, &server.uri(), "qr-1")
            .await
            .unwrap(),
        WeixinPollStatus::Scanned
    ));
    match poll_qr_login_status(&client, &server.uri(), "qr-1")
        .await
        .unwrap()
    {
        WeixinPollStatus::Confirmed(c) => {
            assert_eq!(c.bot_token, "tok-final");
            assert_eq!(c.ilink_bot_id, "bot-123");
            assert_eq!(c.base_url, "https://cluster.example");
        }
        other => panic!("expected Confirmed, got {other:?}"),
    }

    // Header contract: every poll must carry iLink-App-ClientVersion.
    let requests = server.received_requests().await.unwrap();
    for req in requests {
        let has_header = req
            .headers
            .get("ilink-app-clientversion")
            .is_some_and(|v| v.to_str().ok() == Some("1"));
        assert!(has_header, "missing iLink-App-ClientVersion header");
    }
}

#[tokio::test]
async fn missing_token_on_confirmed_is_protocol_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ilink/bot/get_qrcode_status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "confirmed",
            "ilink_bot_id": "bot-1",
            // no bot_token
        })))
        .mount(&server)
        .await;
    let err = poll_qr_login_status(&HttpClient::new(), &server.uri(), "qr")
        .await
        .expect_err("should error");
    assert!(matches!(err, WeixinAuthError::Protocol(_)));
}
