use axum::body::Body;
use axum::http::{Request, StatusCode};
use garyx_gateway::build_router;
use garyx_gateway::server::AppStateBuilder;
use garyx_models::config::GaryxConfig;
use serde_json::{Value, json};
use tower::ServiceExt;

const SHORTCUTS_PATH: &str = "/api/commands/shortcuts";
const TEST_GATEWAY_TOKEN: &str = "commands-api-test-token";

fn test_router() -> axum::Router {
    let mut config = GaryxConfig::default();
    config.gateway.auth_token = TEST_GATEWAY_TOKEN.to_owned();
    let state = AppStateBuilder::new(config).build();
    build_router(state)
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn json_request(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {TEST_GATEWAY_TOKEN}"));
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    builder
        .body(body.map_or_else(Body::empty, |v| Body::from(v.to_string())))
        .unwrap()
}

#[tokio::test]
async fn commands_crud_lifecycle() {
    let router = test_router();

    // List empty
    let resp = router
        .clone()
        .oneshot(json_request("GET", SHORTCUTS_PATH, None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let payload = response_json(resp).await;
    assert_eq!(payload["commands"].as_array().unwrap().len(), 0);

    // Create
    let resp = router
        .clone()
        .oneshot(json_request(
            "POST",
            SHORTCUTS_PATH,
            Some(json!({
                "name": "summary",
                "description": "Summarize the thread",
                "prompt": "Please summarize.",
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = response_json(resp).await;
    assert_eq!(created["name"], "summary");
    assert_eq!(created["prompt"], "Please summarize.");

    // List after create
    let resp = router
        .clone()
        .oneshot(json_request("GET", SHORTCUTS_PATH, None))
        .await
        .unwrap();
    let payload = response_json(resp).await;
    assert_eq!(payload["commands"].as_array().unwrap().len(), 1);

    // Update
    let resp = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/commands/shortcuts/summary",
            Some(json!({
                "name": "summary",
                "description": "Updated description",
                "prompt": "New prompt.",
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let updated = response_json(resp).await;
    assert_eq!(updated["description"], "Updated description");

    // Delete
    let resp = router
        .clone()
        .oneshot(json_request(
            "DELETE",
            "/api/commands/shortcuts/summary",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let deleted = response_json(resp).await;
    assert_eq!(deleted["deleted"], true);

    // Verify deleted
    let resp = router
        .oneshot(json_request("GET", SHORTCUTS_PATH, None))
        .await
        .unwrap();
    let payload = response_json(resp).await;
    assert_eq!(payload["commands"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn commands_rejects_duplicate() {
    let router = test_router();

    let body = json!({
        "name": "hello",
        "description": "Say hello",
        "prompt": "Hi!",
    });

    let resp = router
        .clone()
        .oneshot(json_request("POST", SHORTCUTS_PATH, Some(body.clone())))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let resp = router
        .oneshot(json_request("POST", SHORTCUTS_PATH, Some(body)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn commands_rejects_invalid_name() {
    let router = test_router();

    let resp = router
        .oneshot(json_request(
            "POST",
            SHORTCUTS_PATH,
            Some(json!({
                "name": "INVALID NAME!",
                "description": "Bad",
                "prompt": "x",
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn shortcuts_require_prompt() {
    let router = test_router();

    let resp = router
        .oneshot(json_request(
            "POST",
            SHORTCUTS_PATH,
            Some(json!({
                "name": "empty",
                "description": "No prompt",
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn commands_delete_nonexistent_returns_404() {
    let router = test_router();

    let resp = router
        .oneshot(json_request(
            "DELETE",
            "/api/commands/shortcuts/nonexistent",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
