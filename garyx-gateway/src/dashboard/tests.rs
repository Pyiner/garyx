use super::*;
use axum::Router;
use axum::body::Body;
use axum::http::Request;
use garyx_models::config::GaryxConfig;
use tower::ServiceExt;

fn test_state() -> Arc<AppState> {
    crate::server::create_app_state(GaryxConfig::default())
}

fn dashboard_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/overview", axum::routing::get(overview))
        .route("/api/agent-view", axum::routing::get(agent_view))
        .route("/api/logs/tail", axum::routing::get(logs_tail))
        .route("/api/settings", axum::routing::get(settings))
        .route("/api/stream", axum::routing::get(event_stream))
        .with_state(state)
}

#[tokio::test]
async fn test_overview() {
    let state = test_state();
    let router = dashboard_router(state);

    let req = Request::builder()
        .uri("/api/overview")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "running");
    assert!(json["uptime_seconds"].is_number());
    assert!(json["gateway"].is_object());
    assert_eq!(json["active_runs"], 0);
    assert!(json["mcp_metrics"]["mcp_tool_calls_total"].is_array());
    assert!(json["mcp_metrics"]["mcp_tool_duration_ms"].is_array());
    assert!(json["delivery_target_metrics"]["cache_hits"].is_number());
    assert!(json["delivery_target_metrics"]["store_hits"].is_number());
    assert!(json["delivery_target_metrics"]["store_misses"].is_number());
    assert!(json["delivery_target_metrics"]["by_target"]["last_target"].is_number());
    assert!(json["delivery_target_metrics"]["by_target"]["thread_target"].is_number());
    assert!(json["delivery_target_metrics"]["by_target"]["explicit_target"].is_number());
    assert!(json["delivery_target_metrics"]["by_channel"].is_array());
    assert!(json["delivery_target_metrics"]["by_account"].is_array());
    assert!(json["delivery_target_metrics"]["recovery_duration_ms"]["count"].is_number());
    assert!(json["delivery_target_metrics"]["recovery_duration_ms"]["avg_ms"].is_number());
    assert!(json["delivery_target_metrics"]["recovery_duration_ms"]["p50_ms"].is_number());
    assert!(json["delivery_target_metrics"]["recovery_duration_ms"]["p95_ms"].is_number());
    assert!(json["delivery_target_metrics"]["recovery_duration_ms"]["p99_ms"].is_number());
}

#[tokio::test]
async fn test_agent_view_no_bridge() {
    let state = test_state();
    let router = dashboard_router(state);

    let req = Request::builder()
        .uri("/api/agent-view")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["bridge_ready"], false);
    assert!(json["providers"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_logs_tail_missing_file() {
    let state = test_state();
    let router = dashboard_router(state);

    let req = Request::builder()
        .uri("/api/logs/tail?lines=10&path=/tmp/garyx_test_nonexistent_xyz.log")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json["error"]
            .as_str()
            .unwrap_or_default()
            .contains("outside allowed log directory"),
        "expected error field, got: {json}"
    );
}

#[tokio::test]
async fn test_logs_tail_with_file() {
    let log_dir = allowed_log_dir(&default_log_path());
    let tmp = log_dir.join("garyx_test_logs_tail.log");
    tokio::fs::create_dir_all(&log_dir).await.unwrap();
    tokio::fs::write(&tmp, "line1\nline2\nline3\nline4\nline5\n")
        .await
        .unwrap();

    let state = test_state();
    let router = dashboard_router(state);

    let uri = format!("/api/logs/tail?lines=3&path={}", tmp.display());
    let req = Request::builder().uri(&uri).body(Body::empty()).unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let lines = json["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "line3");
    assert_eq!(lines[1], "line4");
    assert_eq!(lines[2], "line5");

    let _ = tokio::fs::remove_file(&tmp).await;
}

#[tokio::test]
async fn test_logs_tail_with_pattern() {
    let log_dir = allowed_log_dir(&default_log_path());
    let tmp = log_dir.join("garyx_test_logs_pattern.log");
    tokio::fs::create_dir_all(&log_dir).await.unwrap();
    tokio::fs::write(&tmp, "INFO starting\nERROR failed\nINFO done\nERROR oops\n")
        .await
        .unwrap();

    let state = test_state();
    let router = dashboard_router(state);

    let uri = format!(
        "/api/logs/tail?lines=100&pattern=ERROR&path={}",
        tmp.display()
    );
    let req = Request::builder().uri(&uri).body(Body::empty()).unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let lines = json["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].as_str().unwrap().contains("ERROR"));

    let _ = tokio::fs::remove_file(&tmp).await;
}

#[tokio::test]
async fn test_logs_tail_invalid_regex() {
    let log_dir = allowed_log_dir(&default_log_path());
    let tmp = log_dir.join("garyx_test_logs_badregex.log");
    tokio::fs::create_dir_all(&log_dir).await.unwrap();
    tokio::fs::write(&tmp, "line1\n").await.unwrap();

    let state = test_state();
    let router = dashboard_router(state);

    let uri = format!("/api/logs/tail?pattern=%5Binvalid&path={}", tmp.display());
    let req = Request::builder().uri(&uri).body(Body::empty()).unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["error"].as_str().unwrap().contains("invalid regex"));

    let _ = tokio::fs::remove_file(&tmp).await;
}

#[tokio::test]
async fn test_settings_returns_raw_config() {
    let mut config = GaryxConfig::default();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "test".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "secret-bot-token".to_owned(),
                    enabled: true,
                    name: None,
                    agent_id: "claude".to_owned(),
                    workspace_dir: None,
                    owner_target: None,
                    groups: Default::default(),
                },
            ),
        );

    let state = crate::server::create_app_state(config);
    let router = dashboard_router(state);

    let req = Request::builder()
        .uri("/api/settings")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    let tg_account = &json["channels"]["telegram"]["accounts"]["test"];
    assert_eq!(tg_account["config"]["token"], "secret-bot-token");
    assert_eq!(tg_account["agent_id"], "claude");
}
