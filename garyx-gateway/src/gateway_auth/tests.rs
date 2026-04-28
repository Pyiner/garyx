use super::*;
use crate::server::create_app_state;
use garyx_models::config::GaryxConfig;

fn auth_state() -> Arc<AppState> {
    let mut cfg = GaryxConfig::default();
    cfg.gateway.auth_token = "secret-token".to_owned();
    create_app_state(cfg)
}

#[test]
fn request_authorized_rejects_missing_configured_token() {
    let state = create_app_state(GaryxConfig::default());
    let headers = HeaderMap::new();
    let uri: Uri = "/api/threads".parse().expect("uri");
    assert!(!request_authorized(&state, &headers, &uri));
}

#[test]
fn request_authorized_ignores_restart_tokens_without_gateway_token() {
    let mut state = (*create_app_state(GaryxConfig::default())).clone_for_test();
    state.ops.restart_tokens = vec!["restart-token".to_owned()];
    let state = Arc::new(state);
    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        "Bearer restart-token".parse().expect("header"),
    );
    let uri: Uri = "/api/threads".parse().expect("uri");
    assert!(!request_authorized(&state, &headers, &uri));
}

#[test]
fn request_authorized_rejects_without_token() {
    let state = auth_state();
    let headers = HeaderMap::new();
    let uri: Uri = "/api/threads".parse().expect("uri");
    assert!(!request_authorized(&state, &headers, &uri));
}

#[test]
fn request_authorized_does_not_bypass_loopback() {
    let state = auth_state();
    let headers = HeaderMap::new();
    let uri: Uri = "/api/threads".parse().expect("uri");
    assert!(!request_authorized(&state, &headers, &uri));
}

#[test]
fn request_authorized_accepts_remote_with_valid_token() {
    let state = auth_state();
    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        "Bearer secret-token".parse().expect("header"),
    );
    let uri: Uri = "/api/threads".parse().expect("uri");
    assert!(request_authorized(&state, &headers, &uri));
}
