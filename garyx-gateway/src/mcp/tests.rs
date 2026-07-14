use super::*;
use garyx_models::config::{ApiAccount, GaryxConfig};
use uuid::Uuid;

fn test_server() -> GaryMcpServer {
    let state = crate::server::create_app_state(GaryxConfig::default());
    GaryMcpServer::new(state)
}

fn insert_telegram_plugin_account(
    config: &mut GaryxConfig,
    account_id: &str,
    account: garyx_models::config::TelegramAccount,
) {
    config
        .channels
        .plugins
        .entry("telegram".to_owned())
        .or_default()
        .accounts
        .insert(
            account_id.to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(&account),
        );
}

fn insert_feishu_plugin_account(
    config: &mut GaryxConfig,
    account_id: &str,
    account: garyx_models::config::FeishuAccount,
) {
    config
        .channels
        .plugins
        .entry("feishu".to_owned())
        .or_default()
        .accounts
        .insert(
            account_id.to_owned(),
            garyx_models::config::feishu_account_to_plugin_entry(&account),
        );
}

fn insert_weixin_plugin_account(
    config: &mut GaryxConfig,
    account_id: &str,
    account: garyx_models::config::WeixinAccount,
) {
    config
        .channels
        .plugins
        .entry("weixin".to_owned())
        .or_default()
        .accounts
        .insert(
            account_id.to_owned(),
            garyx_models::config::weixin_account_to_plugin_entry(&account),
        );
}

// -- status --

#[tokio::test]
async fn test_status_ok() {
    let server = test_server();
    let v = server.status_payload(RunContext::default()).await.unwrap();
    assert_eq!(v["tool"], "status");
    assert_eq!(v["status"], "ok");
    assert!(v["uptime_secs"].is_number());
    assert_eq!(v["threads"]["count"], 0);
    assert!(v["current_context"].is_object());
    assert!(v["current_context"]["thread_id"].is_null());
}

#[tokio::test]
async fn test_status_counts_threads() {
    let state = crate::server::create_app_state(GaryxConfig::default());
    state
        .threads
        .thread_store
        .set("s1::dm::u1", json!({}))
        .await
        .unwrap();
    state
        .threads
        .thread_store
        .set("s2::dm::u2", json!({}))
        .await
        .unwrap();
    let server = GaryMcpServer::new(state);
    let v = server.status_payload(RunContext::default()).await.unwrap();
    assert_eq!(v["threads"]["count"], 2);
}

#[tokio::test]
async fn test_status_lists_api_channels() {
    let mut config = GaryxConfig::default();
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            workspace_mode: None,
        },
    );
    let server = GaryMcpServer::new(crate::server::create_app_state(config));
    let v = server.status_payload(RunContext::default()).await.unwrap();

    let channels = v["channels"].as_array().unwrap();
    assert!(
        channels
            .iter()
            .any(|c| c["channel_type"] == "api" && c["name"] == "main")
    );
}

#[tokio::test]
async fn test_status_lists_weixin_channels() {
    let mut config = GaryxConfig::default();
    insert_weixin_plugin_account(
        &mut config,
        "wx-main",
        garyx_models::config::WeixinAccount {
            token: "wx-token".to_owned(),
            uin: String::new(),
            enabled: true,
            base_url: "https://ilinkai.weixin.qq.com".to_owned(),
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            streaming_update: true,
        },
    );
    let server = GaryMcpServer::new(crate::server::create_app_state(config));
    let v = server.status_payload(RunContext::default()).await.unwrap();

    let channels = v["channels"].as_array().unwrap();
    assert!(
        channels
            .iter()
            .any(|c| c["channel_type"] == "weixin" && c["name"] == "wx-main")
    );
    let available_bots = v["bots"]["available"].as_array().unwrap();
    assert!(
        available_bots
            .iter()
            .any(|bot| bot["bot"] == "weixin:wx-main")
    );
}

#[tokio::test]
async fn test_status_reports_current_and_other_bots() {
    let mut config = GaryxConfig::default();
    insert_telegram_plugin_account(
        &mut config,
        "main",
        garyx_models::config::TelegramAccount {
            token: "token".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            groups: Default::default(),
        },
    );
    insert_feishu_plugin_account(
        &mut config,
        "ops",
        garyx_models::config::FeishuAccount {
            app_id: "app".to_owned(),
            app_secret: "secret".to_owned(),
            enabled: true,
            domain: Default::default(),
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            require_mention: true,
            topic_session_mode: Default::default(),
        },
    );
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            workspace_mode: None,
        },
    );

    let state = crate::server::create_app_state(config);
    state
        .threads
        .thread_store
        .set(
            "thread::bound-bots",
            json!({
                "thread_id": "thread::bound-bots",
                "channel_bindings": [
                    {
                        "channel": "telegram",
                        "account_id": "main",
                        "peer_id": "42",
                        "chat_id": "42",
                        "display_label": "Telegram Main"
                    },
                    {
                        "channel": "feishu",
                        "account_id": "ops",
                        "peer_id": "ou_42",
                        "chat_id": "oc_42",
                        "display_label": "Feishu Ops"
                    }
                ]
            }),
        )
        .await
        .unwrap();
    let server = GaryMcpServer::new(state);

    let v = server
        .status_payload(RunContext {
            thread_id: Some("thread::bound-bots".to_owned()),
            channel: Some("telegram".to_owned()),
            account_id: Some("main".to_owned()),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(v["bots"]["current"]["bot"], "telegram:main");
    assert_eq!(v["bots"]["current"]["source"], "run_context");
    let thread_bound = v["bots"]["thread_bound"].as_array().unwrap();
    assert!(thread_bound.iter().any(|bot| bot["bot"] == "telegram:main"));
    assert!(thread_bound.iter().any(|bot| bot["bot"] == "feishu:ops"));
    let others = v["bots"]["others"].as_array().unwrap();
    assert!(others.iter().any(|bot| bot["bot"] == "feishu:ops"));
    assert!(others.iter().any(|bot| bot["bot"] == "api:main"));
}

#[tokio::test]
async fn test_status_current_context_includes_workspace_for_thread() {
    let state = crate::server::create_app_state(GaryxConfig::default());
    state
        .threads
        .thread_store
        .set(
            "thread::ctx",
            json!({
                "thread_id": "thread::ctx",
                "workspace_dir": "/tmp/status-workspace"
            }),
        )
        .await
        .unwrap();
    let server = GaryMcpServer::new(state);
    let v = server
        .status_payload(RunContext {
            run_id: Some("run-ctx".to_owned()),
            thread_id: Some("thread::ctx".to_owned()),
            channel: Some("macapp".to_owned()),
            account_id: Some("desktop".to_owned()),
        })
        .await
        .unwrap();

    assert_eq!(v["current_context"]["thread_id"], "thread::ctx");
    assert_eq!(v["current_context"]["channel"], "macapp");
    assert_eq!(
        v["current_context"]["workspace_dir"],
        "/tmp/status-workspace"
    );
}

#[tokio::test]
async fn test_mcp_tool_metrics_record_success_and_error() {
    let server = test_server();

    let _ = server
        .status_payload(RunContext::default())
        .await
        .expect("status should succeed");
    server.record_tool_metric("status", "ok", std::time::Duration::from_millis(1));
    server.record_tool_metric("message", "error", std::time::Duration::from_millis(2));

    let metrics = server.app_state.ops.mcp_tool_metrics.snapshot();
    let status_ok = metrics
        .mcp_tool_calls_total
        .iter()
        .find(|m| m.tool == "status" && m.status == "ok")
        .map(|m| m.value)
        .unwrap_or(0);
    let message_error = metrics
        .mcp_tool_calls_total
        .iter()
        .find(|m| m.tool == "message" && m.status == "error")
        .map(|m| m.value)
        .unwrap_or(0);
    assert!(status_ok >= 1, "expected status ok call metric");
    assert!(message_error >= 1, "expected message error call metric");

    let status_duration = metrics
        .mcp_tool_duration_ms
        .iter()
        .find(|m| m.tool == "status")
        .map(|m| m.count)
        .unwrap_or(0);
    let message_duration = metrics
        .mcp_tool_duration_ms
        .iter()
        .find(|m| m.tool == "message")
        .map(|m| m.count)
        .unwrap_or(0);
    assert!(status_duration >= 1, "expected status duration metric");
    assert!(message_duration >= 1, "expected message duration metric");
}

#[test]
fn test_mcp_tool_router_excludes_cron_management() {
    let server = test_server();
    let names = server
        .tool_router
        .list_all()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();

    assert!(names.iter().any(|name| name == "status"));
    assert!(names.iter().any(|name| name == "capsule_create"));
    assert!(names.iter().any(|name| name == "capsule_update"));
    assert!(names.iter().any(|name| name == "capsule_list"));
    assert!(
        !names.iter().any(|name| name == "message"),
        "outbound message sending must stay out of MCP tools: {names:?}"
    );
    assert!(
        !names.iter().any(|name| name == "cron"),
        "scheduled automation management must stay out of MCP tools: {names:?}"
    );
}

#[test]
fn test_mcp_tool_router_does_not_advertise_image_generation() {
    let server = test_server();
    let names = server
        .tool_router
        .list_all()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();

    assert!(
        !names.iter().any(|name| name == "image_gen"),
        "image generation tool must no longer be advertised: {names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|name| name.contains("image_gen") || name.contains("image_generation")),
        "no image-generation aliases should be advertised: {names:?}"
    );
    // Server instructions must also drop the legacy tool name so agents don't
    // discover it through the MCP greeting text.
    let info = ServerHandler::get_info(&server);
    let instructions = info.instructions.unwrap_or_default();
    assert!(
        !instructions.contains("image_gen"),
        "server instructions still mention the removed image_gen tool: {instructions}"
    );
    assert!(
        !instructions.contains("message"),
        "server instructions still mention the removed message tool: {instructions}"
    );
    assert!(instructions.contains("capsule_create"));
    assert!(instructions.contains("capsule_update"));
    assert!(instructions.contains("capsule_list"));
}

#[tokio::test]
async fn test_capsule_create_uses_thread_context_and_derives_agent_from_thread() {
    let server = test_server();
    let thread_id = "thread::capsule-create";
    server
        .app_state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "agent::capsule",
                "provider_type": "codex"
            }),
        )
        .await
        .unwrap();
    let temp = tempfile::tempdir().expect("temp dir");
    let _guard = crate::capsules::tests_support::set_test_capsules_dir_for_test(
        temp.path().join("capsules"),
    );

    let result = tools::capsule::create_inner(
        &server,
        RunContext {
            thread_id: Some(thread_id.to_owned()),
            run_id: Some("run::capsule-create".to_owned()),
            ..Default::default()
        },
        CapsuleCreateParams {
            title: " Demo Capsule ".to_owned(),
            description: Some(" demo description ".to_owned()),
            html: Some("<html><body><h1>Demo</h1></body></html>".to_owned()),
            html_path: None,
        },
    )
    .await
    .expect("create capsule");

    assert_eq!(result["tool"], "capsule_create");
    assert_eq!(result["status"], "ok");
    assert_eq!(result["title"], "Demo Capsule");
    assert_eq!(result["description"], "demo description");
    assert_eq!(result["thread_id"], thread_id);
    assert_eq!(result["run_id"], "run::capsule-create");
    assert_eq!(result["agent_id"], "agent::capsule");
    assert_eq!(result["provider_type"], "codex_app_server");
    assert_eq!(result["favorited"], false);
    let capsule_id = result["capsule_id"].as_str().expect("capsule id");
    assert!(Uuid::parse_str(capsule_id).is_ok());
    assert_eq!(result["open_url"], format!("garyx://capsules/{capsule_id}"));
    assert_eq!(
        result["serve_path"],
        format!("/api/capsules/{capsule_id}/serve")
    );
    assert!(
        crate::capsules::capsule_file_path(capsule_id)
            .unwrap()
            .is_file()
    );

    let record = server
        .app_state
        .ops
        .garyx_db
        .get_capsule(capsule_id)
        .unwrap()
        .expect("stored capsule");
    assert_eq!(record.thread_id.as_deref(), Some(thread_id));
    assert_eq!(record.agent_id.as_deref(), Some("agent::capsule"));
    assert_eq!(record.provider_type.as_deref(), Some("codex_app_server"));
}

#[tokio::test]
async fn test_capsule_create_rejects_conflicting_missing_and_bad_html() {
    let server = test_server();
    let thread_id = "thread::capsule-reject";
    server
        .app_state
        .threads
        .thread_store
        .set(thread_id, json!({ "thread_id": thread_id }))
        .await
        .unwrap();
    let run_ctx = RunContext {
        thread_id: Some(thread_id.to_owned()),
        ..Default::default()
    };
    let both = tools::capsule::create_inner(
        &server,
        run_ctx.clone(),
        CapsuleCreateParams {
            title: "Bad".to_owned(),
            description: None,
            html: Some("<html></html>".to_owned()),
            html_path: Some("/tmp/test.html".to_owned()),
        },
    )
    .await
    .expect_err("both html and html_path rejected");
    assert!(both.contains("exactly one"));

    let neither = tools::capsule::create_inner(
        &server,
        run_ctx.clone(),
        CapsuleCreateParams {
            title: "Bad".to_owned(),
            description: None,
            html: None,
            html_path: None,
        },
    )
    .await
    .expect_err("missing html rejected");
    assert!(neither.contains("exactly one"));

    let bad_html = tools::capsule::create_inner(
        &server,
        run_ctx.clone(),
        CapsuleCreateParams {
            title: "Bad".to_owned(),
            description: None,
            html: Some("<img src=\"asset.png\">".to_owned()),
            html_path: None,
        },
    )
    .await
    .expect_err("relative html rejected");
    assert!(bad_html.contains("self-contained"));

    let oversized = tools::capsule::create_inner(
        &server,
        run_ctx,
        CapsuleCreateParams {
            title: "Bad".to_owned(),
            description: None,
            html: Some("a".repeat(crate::capsules::CAPSULE_MAX_HTML_BYTES + 1)),
            html_path: None,
        },
    )
    .await
    .expect_err("oversized html rejected");
    assert!(oversized.contains("exceeds"));
}

#[tokio::test]
async fn test_capsule_create_reads_absolute_html_path() {
    let server = test_server();
    let thread_id = "thread::capsule-html-path";
    server
        .app_state
        .threads
        .thread_store
        .set(thread_id, json!({ "thread_id": thread_id }))
        .await
        .unwrap();
    let temp = tempfile::tempdir().expect("temp dir");
    let _guard = crate::capsules::tests_support::set_test_capsules_dir_for_test(
        temp.path().join("capsules"),
    );
    let html_path = temp.path().join("input.html");
    std::fs::write(&html_path, "<html><body>from path</body></html>").expect("write html path");

    let result = tools::capsule::create_inner(
        &server,
        RunContext {
            thread_id: Some(thread_id.to_owned()),
            ..Default::default()
        },
        CapsuleCreateParams {
            title: "Path Capsule".to_owned(),
            description: None,
            html: None,
            html_path: Some(html_path.to_string_lossy().into_owned()),
        },
    )
    .await
    .expect("create capsule from html_path");

    let capsule_id = result["capsule_id"].as_str().unwrap();
    let stored = std::fs::read_to_string(crate::capsules::capsule_file_path(capsule_id).unwrap())
        .expect("stored capsule file");
    assert!(stored.contains("from path"));
}

#[tokio::test]
async fn test_capsule_update_rewrites_file_and_list_filters_thread() {
    let server = test_server();
    let temp = tempfile::tempdir().expect("temp dir");
    let _guard = crate::capsules::tests_support::set_test_capsules_dir_for_test(
        temp.path().join("capsules"),
    );
    for thread_id in ["thread::capsule-a", "thread::capsule-b"] {
        server
            .app_state
            .threads
            .thread_store
            .set(thread_id, json!({ "thread_id": thread_id }))
            .await
            .unwrap();
    }

    let first = tools::capsule::create_inner(
        &server,
        RunContext {
            thread_id: Some("thread::capsule-a".to_owned()),
            ..Default::default()
        },
        CapsuleCreateParams {
            title: "First".to_owned(),
            description: None,
            html: Some("<html><body>first</body></html>".to_owned()),
            html_path: None,
        },
    )
    .await
    .expect("create first");
    let second = tools::capsule::create_inner(
        &server,
        RunContext {
            thread_id: Some("thread::capsule-b".to_owned()),
            ..Default::default()
        },
        CapsuleCreateParams {
            title: "Second".to_owned(),
            description: None,
            html: Some("<html><body>second</body></html>".to_owned()),
            html_path: None,
        },
    )
    .await
    .expect("create second");
    assert_ne!(first["capsule_id"], second["capsule_id"]);
    let capsule_id = first["capsule_id"].as_str().unwrap();
    server
        .app_state
        .ops
        .garyx_db
        .set_capsule_favorite(capsule_id, true)
        .expect("favorite first capsule")
        .expect("first capsule exists");

    let updated = tools::capsule::update_inner(
        &server,
        RunContext::default(),
        CapsuleUpdateParams {
            capsule_id: capsule_id.to_owned(),
            title: Some("First Updated".to_owned()),
            description: Some("updated".to_owned()),
            html: Some("<html><body>updated</body></html>".to_owned()),
            html_path: None,
        },
    )
    .await
    .expect("update capsule");
    assert_eq!(updated["tool"], "capsule_update");
    assert_eq!(updated["revision"], 2);
    assert_eq!(updated["favorited"], true);
    let file = std::fs::read_to_string(crate::capsules::capsule_file_path(capsule_id).unwrap())
        .expect("capsule file");
    assert!(file.contains("updated"));

    let listed = tools::capsule::list_inner(
        &server,
        RunContext {
            thread_id: Some("thread::capsule-a".to_owned()),
            ..Default::default()
        },
    )
    .await
    .expect("list capsules");
    let capsules = listed["capsules"].as_array().unwrap();
    assert_eq!(capsules.len(), 1);
    assert_eq!(capsules[0]["id"], capsule_id);
    assert_eq!(capsules[0]["favorited"], true);
}

#[test]
fn test_capsule_update_schema_has_no_favorite_write_field() {
    let schema = serde_json::to_value(schemars::schema_for!(CapsuleUpdateParams))
        .expect("serialize capsule update schema");
    let properties = schema["properties"]
        .as_object()
        .expect("capsule update schema properties");
    for field in ["favorite", "favorited", "favorited_at", "favoritedAt"] {
        assert!(
            !properties.contains_key(field),
            "capsule_update must not expose favorite write field {field}"
        );
    }

    let parsed: CapsuleUpdateParams = serde_json::from_value(json!({
        "capsule_id": Uuid::new_v4().to_string(),
        "title": "Synthetic update",
        "favorited": true,
    }))
    .expect("unknown favorite input remains ignored");
    assert_eq!(parsed.title.as_deref(), Some("Synthetic update"));
}

#[tokio::test]
async fn test_capsule_favorite_end_to_end_mcp_create_put_and_list() {
    use axum::body::{Body, to_bytes};
    use tower::ServiceExt;

    let state = crate::server::AppStateBuilder::new(crate::test_support::with_gateway_auth(
        GaryxConfig::default(),
    ))
    .build();
    let server = GaryMcpServer::new(state);
    let thread_id = "thread::capsule-favorite-e2e";
    server
        .app_state
        .threads
        .thread_store
        .set(thread_id, json!({ "thread_id": thread_id }))
        .await
        .expect("seed thread");
    let temp = tempfile::tempdir().expect("temp dir");
    let _guard = crate::capsules::tests_support::set_test_capsules_dir_for_test(
        temp.path().join("capsules"),
    );

    let mut created_ids = Vec::new();
    for title in ["Synthetic Favorite A", "Synthetic Favorite B"] {
        let created = tools::capsule::create_inner(
            &server,
            RunContext {
                thread_id: Some(thread_id.to_owned()),
                ..Default::default()
            },
            CapsuleCreateParams {
                title: title.to_owned(),
                description: None,
                html: Some(format!("<html><body>{title}</body></html>")),
                html_path: None,
            },
        )
        .await
        .expect("create capsule through MCP implementation");
        created_ids.push(created["capsule_id"].as_str().unwrap().to_owned());
    }

    let router = crate::route_graph::build_router(server.app_state.clone());
    let favorite = crate::test_support::authed_request()
        .method("PUT")
        .uri(format!("/api/capsules/{}/favorite", created_ids[0]))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(favorite).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    let list = crate::test_support::authed_request()
        .uri("/api/capsules")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(list).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let capsules = payload["capsules"].as_array().expect("capsule list");
    let first = capsules
        .iter()
        .find(|capsule| capsule["id"] == created_ids[0])
        .expect("favorited capsule listed");
    let second = capsules
        .iter()
        .find(|capsule| capsule["id"] == created_ids[1])
        .expect("unfavorited capsule listed");
    assert!(first["favorited_at"].is_string());
    assert!(second["favorited_at"].is_null());
    println!(
        "capsule favorite e2e: created=2 put_status=200 listed=2 favorited_flags=[true,false]"
    );
}

#[test]
fn test_mcp_image_gen_route_is_absent() {
    let server = test_server();
    assert!(
        !server.tool_router.has_route("image_gen"),
        "image_gen route must not be registered on the MCP tool router"
    );
    assert!(
        server.tool_router.get("image_gen").is_none(),
        "image_gen must not resolve through the MCP tool router"
    );
}

#[tokio::test]
async fn test_search_ok() {
    let server = test_server();
    let result = server
        .search(Parameters(SearchParams {
            query: "what is rust programming language".to_owned(),
        }))
        .await;
    assert!(result.is_ok());
    let v: Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert_eq!(v["tool"], "search");
    assert_eq!(v["query"], "what is rust programming language");
}

#[tokio::test]
async fn test_search_missing_key_reports_config_hint() {
    let server = test_server();
    let result = server
        .search(Parameters(SearchParams {
            query: "test query".to_owned(),
        }))
        .await
        .expect("search should return wrapper response");

    let v: Value = serde_json::from_str(&result).expect("valid search json");
    if GaryMcpServer::resolve_search_api_key("").is_some() {
        let status = v["status"].as_str().unwrap_or_default();
        assert!(
            status == "ok" || status == "error",
            "unexpected search status when API key is configured: {status}"
        );
    } else {
        assert_eq!(v["status"], "error");
        let err = v["result"]["error"].as_str().unwrap_or_default();
        assert!(
            err.contains("garyx.json") && err.contains("gateway.search.api_key"),
            "expected config hint in missing-key error, got: {err}"
        );
    }
}

#[tokio::test]
async fn test_search_empty_query() {
    let server = test_server();
    let result = server
        .search(Parameters(SearchParams {
            query: "   ".to_owned(),
        }))
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("query is required"));
}

#[test]
fn test_extract_grounding_sources() {
    let response = json!({
        "candidates": [{
            "content": {
                "parts": [{ "text": "Rust is a systems programming language." }]
            },
            "groundingMetadata": {
                "groundingChunks": [
                    { "web": { "uri": "https://www.rust-lang.org/", "title": "Rust Programming Language" } },
                    { "web": { "uri": "https://en.wikipedia.org/wiki/Rust_(programming_language)", "title": "Rust (programming language)" } }
                ]
            }
        }]
    });

    let sources = GaryMcpServer::extract_grounding_sources(&response);
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0]["url"], "https://www.rust-lang.org/");
    assert_eq!(sources[0]["title"], "Rust Programming Language");
    assert_eq!(
        sources[1]["url"],
        "https://en.wikipedia.org/wiki/Rust_(programming_language)"
    );
}

#[test]
fn test_extract_grounding_sources_empty() {
    let response = json!({
        "candidates": [{
            "content": {
                "parts": [{ "text": "some answer" }]
            }
        }]
    });
    let sources = GaryMcpServer::extract_grounding_sources(&response);
    assert!(sources.is_empty());
}

#[test]
fn test_server_info() {
    let server = test_server();
    let info = server.get_info();
    assert_eq!(info.server_info.name, "gary-mcp");
    assert!(info.capabilities.tools.is_some());
    let instructions = info.instructions.expect("server instructions");
    assert!(!instructions.contains("rebind_current_channel"));
    assert!(!instructions.contains("cron"));
    assert!(!instructions.contains("garyx automation"));
    assert!(!instructions.contains("restart"));
    assert!(!instructions.contains("speak_to_agent"));
}

// -- factory --

#[test]
fn test_create_mcp_service() {
    let state = crate::server::create_app_state(GaryxConfig::default());
    let _service = create_mcp_service(state, CancellationToken::new());
}

#[test]
fn test_decode_mcp_path_context_extracts_thread_and_run_ids() {
    let (thread_id, run_id) = decode_mcp_path_context("/mcp/thread%3A%3Aalpha/run-42");
    assert_eq!(thread_id.as_deref(), Some("thread::alpha"));
    assert_eq!(run_id.as_deref(), Some("run-42"));
}

#[test]
fn test_decode_mcp_path_context_extracts_ids_after_auth_token() {
    let (thread_id, run_id) = decode_mcp_path_context(
        "/mcp/auth/secret%2Ftoken/thread%3A%3Aalpha/run%2F1",
    );
    assert_eq!(thread_id.as_deref(), Some("thread::alpha"));
    assert_eq!(run_id.as_deref(), Some("run/1"));
}

#[test]
fn test_decode_mcp_path_context_handles_thread_only_paths() {
    let (thread_id, run_id) = decode_mcp_path_context("/mcp/thread%3A%3Aalpha");
    assert_eq!(thread_id.as_deref(), Some("thread::alpha"));
    assert_eq!(run_id, None);
}

#[test]
fn test_decode_mcp_path_context_falls_back_to_raw_segment_on_bad_percent_encoding() {
    let (thread_id, run_id) = decode_mcp_path_context("/mcp/thread%ZZ/run-42");
    assert_eq!(thread_id.as_deref(), Some("thread%ZZ"));
    assert_eq!(run_id.as_deref(), Some("run-42"));
}
