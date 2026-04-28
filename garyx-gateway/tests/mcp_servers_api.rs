use axum::body::Body;
use axum::http::{Request, StatusCode};
use garyx_gateway::build_router;
use garyx_gateway::server::AppStateBuilder;
use garyx_models::config::GaryxConfig;
use serde_json::{Value, json};
use tower::ServiceExt;

const TEST_GATEWAY_TOKEN: &str = "mcp-servers-api-test-token";

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
async fn mcp_servers_crud_lifecycle() {
    let router = test_router();

    // List empty
    let resp = router
        .clone()
        .oneshot(json_request("GET", "/api/mcp-servers", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let payload = response_json(resp).await;
    assert_eq!(payload["servers"].as_array().unwrap().len(), 0);

    // Create
    let resp = router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/mcp-servers",
            Some(json!({
                "name": "test-server",
                "command": "npx",
                "args": ["-y", "@test/mcp-server"],
                "env": { "API_KEY": "test-key" },
                "enabled": true,
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = response_json(resp).await;
    assert_eq!(created["name"], "test-server");
    assert_eq!(created["command"], "npx");
    assert_eq!(created["enabled"], true);

    // List after create
    let resp = router
        .clone()
        .oneshot(json_request("GET", "/api/mcp-servers", None))
        .await
        .unwrap();
    let payload = response_json(resp).await;
    assert_eq!(payload["servers"].as_array().unwrap().len(), 1);

    // Toggle disable
    let resp = router
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/mcp-servers/test-server/toggle",
            Some(json!({ "enabled": false })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let toggled = response_json(resp).await;
    assert_eq!(toggled["enabled"], false);

    // Update
    let resp = router
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/mcp-servers/test-server",
            Some(json!({
                "name": "test-server",
                "command": "node",
                "args": ["server.js"],
                "env": {},
                "enabled": true,
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let updated = response_json(resp).await;
    assert_eq!(updated["command"], "node");

    // Delete
    let resp = router
        .clone()
        .oneshot(json_request("DELETE", "/api/mcp-servers/test-server", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify deleted
    let resp = router
        .oneshot(json_request("GET", "/api/mcp-servers", None))
        .await
        .unwrap();
    let payload = response_json(resp).await;
    assert_eq!(payload["servers"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn mcp_servers_rejects_duplicate() {
    let router = test_router();

    let body = json!({
        "name": "dup-server",
        "command": "npx",
        "args": [],
        "env": {},
    });

    let resp = router
        .clone()
        .oneshot(json_request("POST", "/api/mcp-servers", Some(body.clone())))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let resp = router
        .oneshot(json_request("POST", "/api/mcp-servers", Some(body)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn mcp_servers_rejects_invalid_name() {
    let router = test_router();

    let resp = router
        .oneshot(json_request(
            "POST",
            "/api/mcp-servers",
            Some(json!({
                "name": "invalid name!",
                "command": "test",
                "args": [],
                "env": {},
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn mcp_servers_rejects_empty_command() {
    let router = test_router();

    let resp = router
        .oneshot(json_request(
            "POST",
            "/api/mcp-servers",
            Some(json!({
                "name": "no-cmd",
                "command": "",
                "args": [],
                "env": {},
            })),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn mcp_servers_delete_nonexistent_returns_404() {
    let router = test_router();

    let resp = router
        .oneshot(json_request("DELETE", "/api/mcp-servers/ghost", None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
