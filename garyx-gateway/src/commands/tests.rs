use super::*;
use axum::body::Body;
use axum::http::{Method, StatusCode};
use garyx_models::config::GaryxConfig;
use garyx_models::config::SlashCommand;
use tower::ServiceExt;

async fn request_json(
    router: axum::Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = crate::test_support::authed_request()
        .method(method)
        .uri(uri);
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let request = builder
        .body(match body {
            Some(value) => Body::from(serde_json::to_vec(&value).unwrap()),
            None => Body::empty(),
        })
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, payload)
}

#[tokio::test]
async fn test_commands_route_hides_channel_native_by_default() {
    let mut config = crate::test_support::with_gateway_auth(GaryxConfig::default());
    config.commands.push(SlashCommand {
        name: "summary".to_owned(),
        description: "Summarize the thread".to_owned(),
        prompt: Some("Please summarize.".to_owned()),
        skill_id: None,
    });
    let state = crate::server::create_app_state(config);
    let router = crate::route_graph::build_router(state);

    let (status, payload) = request_json(router, Method::GET, "/api/commands", None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["version"], 1);
    assert!(payload["revision"].as_str().unwrap().starts_with("v1:"));
    let names = payload["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["summary"]);
    assert_eq!(payload["commands"][0]["kind"], "shortcut");
}

#[tokio::test]
async fn test_commands_route_returns_channel_native_for_plugin_surface() {
    let mut config = crate::test_support::with_gateway_auth(GaryxConfig::default());
    config.commands.push(SlashCommand {
        name: "summary".to_owned(),
        description: "Summarize the thread".to_owned(),
        prompt: Some("Please summarize.".to_owned()),
        skill_id: None,
    });
    let state = crate::server::create_app_state(config);
    let router = crate::route_graph::build_router(state);

    let (status, payload) = request_json(
        router,
        Method::GET,
        "/api/commands?surface=plugin&channel=telegram&account_id=main",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let commands = payload["commands"].as_array().unwrap();
    assert!(
        commands
            .iter()
            .any(|entry| entry["name"] == "newthread" && entry["kind"] == "channel_native")
    );
    assert!(
        commands
            .iter()
            .any(|entry| entry["name"] == "summary" && entry["kind"] == "shortcut")
    );
}

#[tokio::test]
async fn test_shortcut_create_rejects_channel_native_collision() {
    let state = crate::server::create_app_state(crate::test_support::with_gateway_auth(
        GaryxConfig::default(),
    ));
    let router = crate::route_graph::build_router(state);

    let (status, payload) = request_json(
        router,
        Method::POST,
        "/api/commands/shortcuts",
        Some(json!({
            "name": "loop",
            "description": "Custom loop",
            "prompt": "custom loop prompt"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(payload["error"], "reserved_command_name");
}

#[tokio::test]
async fn test_shortcuts_route_lists_only_valid_prompt_shortcuts() {
    let mut config = crate::test_support::with_gateway_auth(GaryxConfig::default());
    config.commands.push(SlashCommand {
        name: "summary".to_owned(),
        description: "Summarize".to_owned(),
        prompt: Some("Summarize the thread.".to_owned()),
        skill_id: Some("legacy-skill".to_owned()),
    });
    config.commands.push(SlashCommand {
        name: "loop".to_owned(),
        description: "Reserved".to_owned(),
        prompt: Some("Reserved shortcut should be hidden.".to_owned()),
        skill_id: None,
    });
    config.commands.push(SlashCommand {
        name: "triage".to_owned(),
        description: "Skill-only legacy command".to_owned(),
        prompt: None,
        skill_id: Some("legacy-skill".to_owned()),
    });
    let state = crate::server::create_app_state(config);
    let router = crate::route_graph::build_router(state);

    let (status, payload) =
        request_json(router, Method::GET, "/api/commands/shortcuts", None).await;

    assert_eq!(status, StatusCode::OK);
    let commands = payload["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0]["name"], "summary");
    assert!(commands[0].get("skill_id").is_none());
}
