use super::*;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use garyx_channels::{
    ChannelDispatcher, ChannelDispatcherImpl, ChannelInfo, FeishuSender, OutboundMessage,
};
use garyx_models::config::{ApiAccount, GaryxConfig};
use garyx_models::routing::DeliveryContext;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Default)]
struct RecordingDispatcher {
    calls: std::sync::Mutex<Vec<OutboundMessage>>,
    message_ids: Vec<String>,
}

impl RecordingDispatcher {
    fn with_message_ids(message_ids: &[&str]) -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            message_ids: message_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }

    fn calls(&self) -> Vec<OutboundMessage> {
        self.calls
            .lock()
            .expect("recording dispatcher lock poisoned")
            .clone()
    }
}

#[async_trait]
impl ChannelDispatcher for RecordingDispatcher {
    async fn send_message(
        &self,
        request: OutboundMessage,
    ) -> Result<garyx_channels::SendMessageResult, garyx_channels::ChannelError> {
        self.calls
            .lock()
            .expect("recording dispatcher lock poisoned")
            .push(request);
        Ok(garyx_channels::SendMessageResult {
            message_ids: self.message_ids.clone(),
        })
    }

    fn available_channels(&self) -> Vec<ChannelInfo> {
        vec![ChannelInfo {
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            is_running: true,
        }]
    }
}

fn test_server() -> GaryMcpServer {
    let state = crate::server::create_app_state(GaryxConfig::default());
    GaryMcpServer::new(state)
}

fn test_server_with_dispatcher(dispatcher: Arc<dyn ChannelDispatcher>) -> GaryMcpServer {
    let state = crate::server::create_app_state(GaryxConfig::default());
    state.replace_channel_dispatcher(dispatcher);
    GaryMcpServer::new(state)
}

fn test_server_with_dispatcher_and_config(
    dispatcher: Arc<dyn ChannelDispatcher>,
    config: GaryxConfig,
) -> GaryMcpServer {
    let state = crate::server::create_app_state(config);
    state.replace_channel_dispatcher(dispatcher);
    GaryMcpServer::new(state)
}

fn test_server_with_config(config: GaryxConfig) -> GaryMcpServer {
    let state = crate::server::create_app_state(config);
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
            from_id: Some("user-1".to_owned()),
            delivery_thread_id: Some("topic-1".to_owned()),
            auth_token: None,
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

#[test]
fn test_message_params_camel_case_aliases() {
    let parsed: MessageParams = serde_json::from_value(json!({
        "action": "send",
        "target": "42",
        "text": "hello",
        "file": "/tmp/report.pdf",
        "accountId": "main",
        "replyTo": "7",
        "runId": "rid-1"
    }))
    .unwrap();

    assert_eq!(parsed.file.as_deref(), Some("/tmp/report.pdf"));
    assert_eq!(parsed.account_id.as_deref(), Some("main"));
    assert_eq!(parsed.reply_to.as_deref(), Some("7"));
    assert_eq!(parsed.run_id.as_deref(), Some("rid-1"));
}

// -- search --

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

// -- message --

#[test]
fn test_format_scheduled_message() {
    let formatted = GaryMcpServer::format_scheduled_message("hello", Some("cron::daily"));
    assert_eq!(formatted, "#cron::daily\nhello");
}

#[tokio::test]
async fn test_execute_message_resolves_thread_target_and_dispatches() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let server = test_server_with_dispatcher(dispatcher.clone());

    {
        let mut router = server.app_state.threads.router.lock().await;
        router.set_last_delivery(
            "cron::daily",
            DeliveryContext {
                channel: "telegram".to_owned(),
                account_id: "main".to_owned(),
                chat_id: "42".to_owned(),
                user_id: "42".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "42".to_owned(),
                thread_id: None,
                metadata: HashMap::new(),
            },
        );
    }

    let result = server
        .execute_message(
            RunContext::default(),
            MessageParams {
                action: Some("send".to_owned()),
                target: Some("thread:cron::daily".to_owned()),
                text: Some("scheduled".to_owned()),
                image: None,
                file: None,
                bot: None,
                channel: None,
                account_id: None,
                reply_to: None,
                run_id: None,
                token: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(result["status"], "ok");
    assert_eq!(result["chat_id"], "42");
    assert_eq!(result["thread_id"], "cron::daily");

    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].channel, "telegram");
    assert_eq!(calls[0].account_id, "main");
    assert_eq!(calls[0].chat_id, "42");
    assert_eq!(calls[0].text_content(), Some("#cron::daily\nscheduled"));
}

#[tokio::test]
async fn test_execute_message_recovers_thread_target_from_persisted_delivery() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let server = test_server_with_dispatcher(dispatcher.clone());

    server
        .app_state
        .threads
        .thread_store
        .set(
            "cron::daily",
            json!({
                "lastChannel": "telegram",
                "lastTo": "84",
                "lastAccountId": "main",
                "lastUpdatedAt": "2026-03-01T12:00:00Z",
            }),
        )
        .await
        .unwrap();

    let result = server
        .execute_message(
            RunContext::default(),
            MessageParams {
                action: Some("send".to_owned()),
                target: Some("thread:cron::daily".to_owned()),
                text: Some("scheduled".to_owned()),
                image: None,
                file: None,
                bot: None,
                channel: None,
                account_id: None,
                reply_to: None,
                run_id: None,
                token: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(result["status"], "ok");
    assert_eq!(result["chat_id"], "84");
    assert_eq!(result["thread_id"], "cron::daily");

    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].channel, "telegram");
    assert_eq!(calls[0].account_id, "main");
    assert_eq!(calls[0].chat_id, "84");
    assert_eq!(calls[0].text_content(), Some("#cron::daily\nscheduled"));
}

#[tokio::test]
async fn test_execute_message_fallback_to_run_context() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let server = test_server_with_dispatcher(dispatcher.clone());

    let run_ctx = RunContext {
        channel: Some("telegram".to_owned()),
        account_id: Some("main".to_owned()),
        from_id: Some("99".to_owned()),
        ..Default::default()
    };

    let result = server
        .execute_message(
            run_ctx,
            MessageParams {
                action: None,
                target: Some("123".to_owned()),
                text: Some("hello".to_owned()),
                image: None,
                file: None,
                bot: None,
                channel: None,
                account_id: None,
                reply_to: None,
                run_id: None,
                token: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(result["status"], "ok");
    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].chat_id, "123");
}

#[tokio::test]
async fn test_execute_message_without_target_prefers_current_thread_over_last() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let server = test_server_with_dispatcher(dispatcher.clone());

    {
        let mut router = server.app_state.threads.router.lock().await;
        router.set_last_delivery(
            "thread::other",
            DeliveryContext {
                channel: "telegram".to_owned(),
                account_id: "main".to_owned(),
                chat_id: "other-chat".to_owned(),
                user_id: "other-chat".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "other-chat".to_owned(),
                thread_id: None,
                metadata: HashMap::new(),
            },
        );
        router.set_last_delivery(
            "thread::current",
            DeliveryContext {
                channel: "telegram".to_owned(),
                account_id: "office_codex".to_owned(),
                chat_id: "current-chat".to_owned(),
                user_id: "current-chat".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "current-chat".to_owned(),
                thread_id: None,
                metadata: HashMap::new(),
            },
        );
    }

    let run_ctx = RunContext {
        thread_id: Some("thread::current".to_owned()),
        ..Default::default()
    };

    let result = server
        .execute_message(
            run_ctx,
            MessageParams {
                action: None,
                target: None,
                text: Some("hello current".to_owned()),
                image: None,
                file: None,
                bot: None,
                channel: None,
                account_id: None,
                reply_to: None,
                run_id: None,
                token: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(result["status"], "ok");
    assert_eq!(result["thread_id"], "thread::current");
    assert_eq!(result["bot"], "telegram:office_codex");

    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].account_id, "office_codex");
    assert_eq!(calls[0].chat_id, "current-chat");
    assert_eq!(calls[0].text_content(), Some("hello current"));
}

#[tokio::test]
async fn test_execute_message_without_target_or_thread_errors() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let server = test_server_with_dispatcher(dispatcher.clone());

    let error = server
        .execute_message(
            RunContext::default(),
            MessageParams {
                action: None,
                target: None,
                text: Some("hello".to_owned()),
                image: None,
                file: None,
                bot: None,
                channel: None,
                account_id: None,
                reply_to: None,
                run_id: None,
                token: None,
            },
        )
        .await
        .expect_err("missing thread context should error");

    assert!(error.contains("current thread context or explicit target"));
    assert!(dispatcher.calls().is_empty());
}

#[tokio::test]
async fn test_execute_message_uses_requested_bot_for_explicit_target() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let mut config = GaryxConfig::default();
    insert_telegram_plugin_account(
        &mut config,
        "main",
        garyx_models::config::TelegramAccount {
            token: "main-token".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            groups: Default::default(),
        },
    );
    insert_telegram_plugin_account(
        &mut config,
        "ops",
        garyx_models::config::TelegramAccount {
            token: "ops-token".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            groups: Default::default(),
        },
    );
    let server = test_server_with_dispatcher_and_config(dispatcher.clone(), config);

    let result = server
        .execute_message(
            RunContext::default(),
            MessageParams {
                action: Some("send".to_owned()),
                target: Some("123".to_owned()),
                text: Some("hello from ops".to_owned()),
                image: None,
                file: None,
                bot: Some("telegram:ops".to_owned()),
                channel: None,
                account_id: None,
                reply_to: None,
                run_id: None,
                token: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(result["status"], "ok");
    assert_eq!(result["bot"], "telegram:ops");
    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].channel, "telegram");
    assert_eq!(calls[0].account_id, "ops");
    assert_eq!(calls[0].chat_id, "123");
}

#[tokio::test]
async fn test_execute_message_uses_requested_bot_main_endpoint_when_target_omitted() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let mut config = GaryxConfig::default();
    insert_telegram_plugin_account(
        &mut config,
        "main",
        garyx_models::config::TelegramAccount {
            token: "main-token".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: Some(garyx_models::config::OwnerTargetConfig {
                target_type: String::new(),
                target_id: "owner-42".to_owned(),
            }),
            groups: Default::default(),
        },
    );
    let server = test_server_with_dispatcher_and_config(dispatcher.clone(), config);

    let run_ctx = RunContext {
        channel: Some("telegram".to_owned()),
        account_id: Some("office_codex".to_owned()),
        from_id: Some("999".to_owned()),
        thread_id: Some("thread::current".to_owned()),
        ..Default::default()
    };

    let result = server
        .execute_message(
            run_ctx,
            MessageParams {
                action: Some("send".to_owned()),
                target: None,
                text: Some("hello owner target".to_owned()),
                image: None,
                file: None,
                bot: Some("telegram:main".to_owned()),
                channel: None,
                account_id: None,
                reply_to: None,
                run_id: None,
                token: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(result["status"], "ok");
    assert_eq!(result["bot"], "telegram:main");
    assert_eq!(result["chat_id"], "owner-42");
    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].channel, "telegram");
    assert_eq!(calls[0].account_id, "main");
    assert_eq!(calls[0].chat_id, "owner-42");
    assert_eq!(calls[0].delivery_target_type, "chat_id");
    assert_eq!(calls[0].delivery_target_id, "owner-42");
}

#[tokio::test]
async fn test_execute_message_errors_when_requested_bot_has_no_main_endpoint_without_target() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let mut config = GaryxConfig::default();
    insert_telegram_plugin_account(
        &mut config,
        "main",
        garyx_models::config::TelegramAccount {
            token: "main-token".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            groups: Default::default(),
        },
    );
    let server = test_server_with_dispatcher_and_config(dispatcher, config);

    let error = server
        .execute_message(
            RunContext::default(),
            MessageParams {
                action: Some("send".to_owned()),
                target: None,
                text: Some("hello owner target".to_owned()),
                image: None,
                file: None,
                bot: Some("telegram:main".to_owned()),
                channel: None,
                account_id: None,
                reply_to: None,
                run_id: None,
                token: None,
            },
        )
        .await
        .unwrap_err();

    assert!(error.contains("has no resolved main endpoint"));
}

#[tokio::test]
async fn test_execute_message_uses_thread_binding_for_requested_bot() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let mut config = GaryxConfig::default();
    insert_telegram_plugin_account(
        &mut config,
        "main",
        garyx_models::config::TelegramAccount {
            token: "main-token".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            groups: Default::default(),
        },
    );
    insert_telegram_plugin_account(
        &mut config,
        "ops",
        garyx_models::config::TelegramAccount {
            token: "ops-token".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            groups: Default::default(),
        },
    );
    let server = test_server_with_dispatcher_and_config(dispatcher.clone(), config);

    server
        .app_state
        .threads
        .thread_store
        .set(
            "thread::multi-bot",
            json!({
                "thread_id": "thread::multi-bot",
                "channel_bindings": [
                    {
                        "channel": "telegram",
                        "account_id": "main",
                        "peer_id": "42",
                        "chat_id": "42",
                        "display_label": "Main Bot"
                    },
                    {
                        "channel": "telegram",
                        "account_id": "ops",
                        "peer_id": "84",
                        "chat_id": "84",
                        "display_label": "Ops Bot"
                    }
                ]
            }),
        )
        .await
        .unwrap();

    {
        let mut router = server.app_state.threads.router.lock().await;
        router.set_last_delivery(
            "thread::multi-bot",
            DeliveryContext {
                channel: "telegram".to_owned(),
                account_id: "main".to_owned(),
                chat_id: "42".to_owned(),
                user_id: "42".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "42".to_owned(),
                thread_id: None,
                metadata: HashMap::new(),
            },
        );
    }

    let result = server
        .execute_message(
            RunContext::default(),
            MessageParams {
                action: Some("send".to_owned()),
                target: Some("thread:thread::multi-bot".to_owned()),
                text: Some("route via ops".to_owned()),
                image: None,
                file: None,
                bot: Some("telegram:ops".to_owned()),
                channel: None,
                account_id: None,
                reply_to: None,
                run_id: None,
                token: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(result["status"], "ok");
    assert_eq!(result["bot"], "telegram:ops");
    assert_eq!(result["chat_id"], "84");
    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].account_id, "ops");
    assert_eq!(calls[0].chat_id, "84");
}

#[tokio::test]
async fn test_execute_message_errors_when_requested_bot_not_bound_to_thread() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let mut config = GaryxConfig::default();
    insert_telegram_plugin_account(
        &mut config,
        "main",
        garyx_models::config::TelegramAccount {
            token: "main-token".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            groups: Default::default(),
        },
    );
    insert_telegram_plugin_account(
        &mut config,
        "ops",
        garyx_models::config::TelegramAccount {
            token: "ops-token".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            groups: Default::default(),
        },
    );
    let server = test_server_with_dispatcher_and_config(dispatcher, config);

    {
        let mut router = server.app_state.threads.router.lock().await;
        router.set_last_delivery(
            "thread::single-bot",
            DeliveryContext {
                channel: "telegram".to_owned(),
                account_id: "main".to_owned(),
                chat_id: "42".to_owned(),
                user_id: "42".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "42".to_owned(),
                thread_id: None,
                metadata: HashMap::new(),
            },
        );
    }

    let error = server
        .execute_message(
            RunContext::default(),
            MessageParams {
                action: Some("send".to_owned()),
                target: Some("thread:thread::single-bot".to_owned()),
                text: Some("route via ops".to_owned()),
                image: None,
                file: None,
                bot: Some("telegram:ops".to_owned()),
                channel: None,
                account_id: None,
                reply_to: None,
                run_id: None,
                token: None,
            },
        )
        .await
        .unwrap_err();

    assert!(error.contains("not bound to bot 'telegram:ops'"));
}

#[tokio::test]
async fn test_execute_message_records_outbound_ids_for_thread_target() {
    let dispatcher = Arc::new(RecordingDispatcher::with_message_ids(&["msg-1", "msg-2"]));
    let server = test_server_with_dispatcher(dispatcher.clone());

    server
        .app_state
        .threads
        .thread_store
        .set("thread::bound", json!({}))
        .await
        .unwrap();

    {
        let mut router = server.app_state.threads.router.lock().await;
        router.set_last_delivery(
            "thread::bound",
            DeliveryContext {
                channel: "telegram".to_owned(),
                account_id: "main".to_owned(),
                chat_id: "42".to_owned(),
                user_id: "42".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "42".to_owned(),
                thread_id: None,
                metadata: HashMap::new(),
            },
        );
    }

    let result = server
        .execute_message(
            RunContext::default(),
            MessageParams {
                action: Some("send".to_owned()),
                target: Some("thread:thread::bound".to_owned()),
                text: Some("mirror me".to_owned()),
                image: None,
                file: None,
                bot: None,
                channel: None,
                account_id: None,
                reply_to: None,
                run_id: Some("run-msg".to_owned()),
                token: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(result["text"], "mirror me");
    assert_eq!(result["message_ids"], json!(["msg-1", "msg-2"]));

    let stored = server
        .app_state
        .threads
        .thread_store
        .get("thread::bound")
        .await
        .unwrap()
        .expect("thread should exist");
    let outbound = stored["outbound_message_ids"]
        .as_array()
        .expect("outbound ids array");
    assert_eq!(outbound.len(), 2);
    assert_eq!(outbound[0]["message_id"], "msg-1");
    assert_eq!(outbound[1]["message_id"], "msg-2");
}

#[tokio::test]
async fn test_execute_message_replies_within_feishu_topic_thread() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth/v3/tenant_access_token/internal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "tenant_access_token": "tenant-token",
            "expire": 7200
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/im/v1/messages/om_root_123/reply"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": { "message_id": "om_reply_123" }
        })))
        .mount(&server)
        .await;

    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_feishu(FeishuSender::new(
        "main".to_owned(),
        "app-123".to_owned(),
        "secret".to_owned(),
        server.uri(),
        true,
    ));
    let server_under_test = test_server_with_dispatcher(Arc::new(dispatcher));

    {
        let mut router = server_under_test.app_state.threads.router.lock().await;
        router.set_last_delivery(
            "thread::feishu-topic",
            DeliveryContext {
                channel: "feishu".to_owned(),
                account_id: "main".to_owned(),
                chat_id: "oc_group_123".to_owned(),
                user_id: "ou_user_123".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "oc_group_123".to_owned(),
                thread_id: Some("oc_group_123:topic:om_root_123".to_owned()),
                metadata: HashMap::new(),
            },
        );
    }

    let result = server_under_test
        .execute_message(
            RunContext::default(),
            MessageParams {
                action: Some("send".to_owned()),
                target: Some("thread:thread::feishu-topic".to_owned()),
                text: Some("hello topic".to_owned()),
                image: None,
                file: None,
                bot: None,
                channel: None,
                account_id: None,
                reply_to: None,
                run_id: Some("run-feishu-topic".to_owned()),
                token: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(result["status"], "ok");
    assert_eq!(result["message_ids"], json!(["om_reply_123"]));

    let requests = server.received_requests().await.expect("received requests");
    let reply_calls = requests
        .iter()
        .filter(|request| request.url.path() == "/im/v1/messages/om_root_123/reply")
        .count();
    assert_eq!(reply_calls, 1);
}

#[tokio::test]
async fn test_execute_message_requires_resolved_target() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let server = test_server_with_dispatcher(dispatcher);

    let result = server
        .execute_message(
            RunContext::default(),
            MessageParams {
                action: None,
                target: None,
                text: Some("hello".to_owned()),
                image: None,
                file: None,
                bot: None,
                channel: None,
                account_id: None,
                reply_to: None,
                run_id: None,
                token: None,
            },
        )
        .await;

    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .contains("current thread context or explicit target")
    );
}

#[tokio::test]
async fn test_execute_message_image_requires_absolute_path() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let server = test_server_with_dispatcher(dispatcher);

    let run_ctx = RunContext {
        channel: Some("telegram".to_owned()),
        account_id: Some("main".to_owned()),
        from_id: Some("42".to_owned()),
        ..Default::default()
    };

    let result = server
        .execute_message(
            run_ctx,
            MessageParams {
                action: Some("send".to_owned()),
                target: Some("42".to_owned()),
                text: Some("caption".to_owned()),
                image: Some("relative/path.png".to_owned()),
                file: None,
                bot: None,
                channel: None,
                account_id: None,
                reply_to: None,
                run_id: None,
                token: None,
            },
        )
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("must be absolute"));
}

#[tokio::test]
async fn test_execute_message_image_unsupported_channel() {
    let server = test_server();
    let image_path = std::env::temp_dir().join(format!(
        "garyx-mcp-unsupported-photo-{}.png",
        Uuid::new_v4()
    ));
    tokio::fs::write(&image_path, b"fake-image-bytes")
        .await
        .expect("write temp image");

    let target = ResolvedMessageTarget {
        channel: "unsupported".to_owned(),
        account_id: "main".to_owned(),
        chat_id: "42".to_owned(),
        delivery_target_type: "chat_id".to_owned(),
        delivery_target_id: "42".to_owned(),
        delivery_thread_id: None,
        thread_id: None,
    };

    let result = server
        .send_image_message_via_api_base(
            &target,
            image_path.to_str().expect("utf-8 image path"),
            Some("caption"),
            None,
            "https://api.telegram.org",
        )
        .await;

    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .contains("only for telegram/weixin/feishu")
    );
    let _ = tokio::fs::remove_file(&image_path).await;
}

#[tokio::test]
async fn test_execute_message_rejects_multiple_attachments() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let server = test_server_with_dispatcher(dispatcher);

    let run_ctx = RunContext {
        channel: Some("telegram".to_owned()),
        account_id: Some("main".to_owned()),
        from_id: Some("42".to_owned()),
        ..Default::default()
    };

    let result = server
        .execute_message(
            run_ctx,
            MessageParams {
                action: Some("send".to_owned()),
                target: Some("42".to_owned()),
                text: Some("caption".to_owned()),
                image: Some("/tmp/one.png".to_owned()),
                file: Some("/tmp/two.pdf".to_owned()),
                bot: None,
                channel: None,
                account_id: None,
                reply_to: None,
                run_id: None,
                token: None,
            },
        )
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("at most one attachment"));
}

#[tokio::test]
async fn test_execute_message_file_requires_absolute_path() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let server = test_server_with_dispatcher(dispatcher);

    let run_ctx = RunContext {
        channel: Some("telegram".to_owned()),
        account_id: Some("main".to_owned()),
        from_id: Some("42".to_owned()),
        ..Default::default()
    };

    let result = server
        .execute_message(
            run_ctx,
            MessageParams {
                action: Some("send".to_owned()),
                target: Some("42".to_owned()),
                text: Some("caption".to_owned()),
                image: None,
                file: Some("relative/report.pdf".to_owned()),
                bot: None,
                channel: None,
                account_id: None,
                reply_to: None,
                run_id: None,
                token: None,
            },
        )
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("must be absolute"));
}

#[test]
fn test_extract_weixin_context_token_from_thread_data_prefers_latest_message() {
    let thread_data = json!({
        "messages": [
            { "metadata": { "context_token": "token-old-meta" } },
            { "context_token": "token-old-direct" },
            { "metadata": { "context_token": "token-newest-meta" } }
        ]
    });
    let token = GaryMcpServer::extract_weixin_context_token_from_thread_data(&thread_data);
    assert_eq!(token.as_deref(), Some("token-newest-meta"));
}

#[tokio::test]
async fn test_send_image_message_via_api_base_returns_message_id_and_records_thread_log() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/botfake-token/sendPhoto"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": {
                "message_id": 1001,
                "chat": {"id": 42, "type": "private"},
                "date": 1700000000
            }
        })))
        .mount(&mock)
        .await;

    let mut config = GaryxConfig::default();
    insert_telegram_plugin_account(
        &mut config,
        "main",
        garyx_models::config::TelegramAccount {
            token: "fake-token".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            groups: HashMap::new(),
        },
    );
    let server = test_server_with_config(config);
    server
        .app_state
        .threads
        .thread_store
        .set("thread::photo", json!({}))
        .await
        .unwrap();

    let image_path = std::env::temp_dir().join(format!("garyx-mcp-photo-{}.png", Uuid::new_v4()));
    tokio::fs::write(&image_path, b"fake-image-bytes")
        .await
        .expect("write temp image");

    let target = ResolvedMessageTarget {
        channel: "telegram".to_owned(),
        account_id: "main".to_owned(),
        chat_id: "42".to_owned(),
        delivery_target_type: "chat_id".to_owned(),
        delivery_target_id: "42".to_owned(),
        delivery_thread_id: None,
        thread_id: Some("thread::photo".to_owned()),
    };

    let message_id = server
        .send_image_message_via_api_base(
            &target,
            image_path.to_str().expect("utf-8 image path"),
            Some("caption"),
            None,
            &mock.uri(),
        )
        .await
        .expect("image send should succeed");

    {
        let mut router = server.app_state.threads.router.lock().await;
        router
            .record_outbound_message_with_thread_log(
                "thread::photo",
                "telegram",
                "main",
                "42",
                None,
                &message_id,
                None,
            )
            .await;
    }

    assert_eq!(message_id, "1001");

    let stored = server
        .app_state
        .threads
        .thread_store
        .get("thread::photo")
        .await
        .unwrap()
        .expect("thread should exist");
    let outbound = stored["outbound_message_ids"]
        .as_array()
        .expect("outbound ids array");
    assert_eq!(outbound.len(), 1);
    assert_eq!(outbound[0]["message_id"], "1001");

    let requests = mock.received_requests().await.expect("received requests");
    assert_eq!(requests.len(), 1);

    let _ = tokio::fs::remove_file(&image_path).await;
}

#[tokio::test]
async fn test_send_image_message_via_api_base_supports_weixin() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ilink/bot/getuploadurl"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ret": 0,
            "upload_param": "up-param"
        })))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload"))
        .respond_with(ResponseTemplate::new(200).insert_header("x-encrypted-param", "dl-param"))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/ilink/bot/sendmessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ret": 0
        })))
        .mount(&mock)
        .await;

    let mut config = GaryxConfig::default();
    insert_weixin_plugin_account(
        &mut config,
        "main",
        garyx_models::config::WeixinAccount {
            token: "weixin-token".to_owned(),
            uin: String::new(),
            enabled: true,
            base_url: mock.uri(),
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            streaming_update: true,
        },
    );
    let server = test_server_with_config(config);

    garyx_channels::weixin::set_context_token("main", "u@im.wechat", "ctx-token").await;

    let image_path =
        std::env::temp_dir().join(format!("garyx-mcp-weixin-photo-{}.png", Uuid::new_v4()));
    tokio::fs::write(&image_path, b"fake-image-bytes")
        .await
        .expect("write temp image");

    let target = ResolvedMessageTarget {
        channel: "weixin".to_owned(),
        account_id: "main".to_owned(),
        chat_id: "u@im.wechat".to_owned(),
        delivery_target_type: "chat_id".to_owned(),
        delivery_target_id: "u@im.wechat".to_owned(),
        delivery_thread_id: None,
        thread_id: None,
    };

    let message_id = server
        .send_image_message_via_api_base(
            &target,
            image_path.to_str().expect("utf-8 image path"),
            Some("caption"),
            None,
            &mock.uri(),
        )
        .await
        .expect("weixin image send should succeed");
    assert!(!message_id.trim().is_empty());

    let requests = mock.received_requests().await.expect("received requests");
    assert!(
        requests
            .iter()
            .any(|req| req.url.path() == "/ilink/bot/getuploadurl")
    );
    assert!(requests.iter().any(|req| req.url.path() == "/upload"));
    assert!(
        requests
            .iter()
            .any(|req| req.url.path() == "/ilink/bot/sendmessage")
    );

    let _ = tokio::fs::remove_file(&image_path).await;
}

#[tokio::test]
async fn test_send_file_message_via_api_base_supports_telegram() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/botfake-token/sendDocument"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": {
                "message_id": 2002,
                "chat": {"id": 42, "type": "private"},
                "date": 1700000000
            }
        })))
        .mount(&mock)
        .await;

    let mut config = GaryxConfig::default();
    insert_telegram_plugin_account(
        &mut config,
        "main",
        garyx_models::config::TelegramAccount {
            token: "fake-token".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            groups: HashMap::new(),
        },
    );
    let server = test_server_with_config(config);

    let file_path = std::env::temp_dir().join(format!("garyx-mcp-doc-{}.pdf", Uuid::new_v4()));
    tokio::fs::write(&file_path, b"fake-pdf-bytes")
        .await
        .expect("write temp file");

    let target = ResolvedMessageTarget {
        channel: "telegram".to_owned(),
        account_id: "main".to_owned(),
        chat_id: "42".to_owned(),
        delivery_target_type: "chat_id".to_owned(),
        delivery_target_id: "42".to_owned(),
        delivery_thread_id: None,
        thread_id: None,
    };

    let message_id = server
        .send_file_message_via_api_base(
            &target,
            file_path.to_str().expect("utf-8 file path"),
            Some("report"),
            None,
            &mock.uri(),
        )
        .await
        .expect("telegram file send should succeed");
    assert_eq!(message_id, "2002");

    let requests = mock.received_requests().await.expect("received requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/botfake-token/sendDocument");

    let _ = tokio::fs::remove_file(&file_path).await;
}

#[tokio::test]
async fn test_send_file_message_via_api_base_supports_weixin() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/ilink/bot/getuploadurl"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ret": 0,
            "upload_param": "up-param"
        })))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload"))
        .respond_with(ResponseTemplate::new(200).insert_header("x-encrypted-param", "dl-param"))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/ilink/bot/sendmessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ret": 0
        })))
        .mount(&mock)
        .await;

    let mut config = GaryxConfig::default();
    insert_weixin_plugin_account(
        &mut config,
        "main",
        garyx_models::config::WeixinAccount {
            token: "weixin-token".to_owned(),
            uin: String::new(),
            enabled: true,
            base_url: mock.uri(),
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            streaming_update: true,
        },
    );
    let server = test_server_with_config(config);

    garyx_channels::weixin::set_context_token("main", "u@im.wechat", "ctx-token").await;

    let file_path =
        std::env::temp_dir().join(format!("garyx-mcp-weixin-file-{}.pdf", Uuid::new_v4()));
    tokio::fs::write(&file_path, b"fake-pdf-bytes")
        .await
        .expect("write temp file");

    let target = ResolvedMessageTarget {
        channel: "weixin".to_owned(),
        account_id: "main".to_owned(),
        chat_id: "u@im.wechat".to_owned(),
        delivery_target_type: "chat_id".to_owned(),
        delivery_target_id: "u@im.wechat".to_owned(),
        delivery_thread_id: None,
        thread_id: None,
    };

    let message_id = server
        .send_file_message_via_api_base(
            &target,
            file_path.to_str().expect("utf-8 file path"),
            Some("report"),
            None,
            &mock.uri(),
        )
        .await
        .expect("weixin file send should succeed");
    assert!(!message_id.trim().is_empty());

    let requests = mock.received_requests().await.expect("received requests");
    assert!(
        requests
            .iter()
            .any(|req| req.url.path() == "/ilink/bot/getuploadurl")
    );
    assert!(requests.iter().any(|req| req.url.path() == "/upload"));
    assert!(
        requests
            .iter()
            .any(|req| req.url.path() == "/ilink/bot/sendmessage")
    );

    let _ = tokio::fs::remove_file(&file_path).await;
}

#[tokio::test]
async fn test_send_image_message_via_api_base_supports_feishu() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "tenant_access_token": "tenant-token",
            "expire": 3600
        })))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/open-apis/im/v1/images"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": { "image_key": "img_v3_abc" }
        })))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/open-apis/im/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": { "message_id": "om_image_123" }
        })))
        .mount(&mock)
        .await;

    let mut config = GaryxConfig::default();
    insert_feishu_plugin_account(
        &mut config,
        "main",
        garyx_models::config::FeishuAccount {
            app_id: "cli_app_id".to_owned(),
            app_secret: "cli_app_secret".to_owned(),
            enabled: true,
            domain: garyx_models::config::FeishuDomain::Feishu,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            require_mention: true,
            topic_session_mode: Default::default(),
        },
    );
    let server = test_server_with_config(config);

    let image_path =
        std::env::temp_dir().join(format!("garyx-mcp-feishu-photo-{}.png", Uuid::new_v4()));
    tokio::fs::write(&image_path, b"fake-image-bytes")
        .await
        .expect("write temp image");

    let target = ResolvedMessageTarget {
        channel: "feishu".to_owned(),
        account_id: "main".to_owned(),
        chat_id: "oc_xxx".to_owned(),
        delivery_target_type: "chat_id".to_owned(),
        delivery_target_id: "oc_xxx".to_owned(),
        delivery_thread_id: None,
        thread_id: None,
    };

    let message_id = server
        .send_image_message_via_api_base(
            &target,
            image_path.to_str().expect("utf-8 image path"),
            None,
            None,
            &format!("{}/open-apis", mock.uri()),
        )
        .await
        .expect("feishu image send should succeed");
    assert_eq!(message_id, "om_image_123");

    let requests = mock.received_requests().await.expect("received requests");
    assert!(
        requests
            .iter()
            .any(|req| req.url.path() == "/open-apis/auth/v3/tenant_access_token/internal")
    );
    assert!(
        requests
            .iter()
            .any(|req| req.url.path() == "/open-apis/im/v1/images")
    );
    assert!(
        requests
            .iter()
            .any(|req| req.url.path() == "/open-apis/im/v1/messages")
    );

    let _ = tokio::fs::remove_file(&image_path).await;
}

#[tokio::test]
async fn test_send_file_message_via_api_base_supports_feishu() {
    let mock = MockServer::start().await;
    let api_base = format!("{}/open-apis", mock.uri());
    Mock::given(method("POST"))
        .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "tenant_access_token": "tenant-token",
            "expire": 3600
        })))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/open-apis/im/v1/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": { "file_key": "file_v3_abc" }
        })))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/open-apis/im/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "code": 0,
            "data": { "message_id": "om_file_123" }
        })))
        .mount(&mock)
        .await;

    let mut config = GaryxConfig::default();
    insert_feishu_plugin_account(
        &mut config,
        "main",
        garyx_models::config::FeishuAccount {
            app_id: "cli_app_id".to_owned(),
            app_secret: "cli_app_secret".to_owned(),
            enabled: true,
            domain: garyx_models::config::FeishuDomain::Feishu,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            require_mention: true,
            topic_session_mode: Default::default(),
        },
    );
    let server = test_server_with_config(config);

    let file_path =
        std::env::temp_dir().join(format!("garyx-mcp-feishu-file-{}.txt", Uuid::new_v4()));
    tokio::fs::write(&file_path, b"fake-file-bytes")
        .await
        .expect("write temp file");

    let target = ResolvedMessageTarget {
        channel: "feishu".to_owned(),
        account_id: "main".to_owned(),
        chat_id: "oc_xxx".to_owned(),
        delivery_target_type: "chat_id".to_owned(),
        delivery_target_id: "oc_xxx".to_owned(),
        delivery_thread_id: None,
        thread_id: None,
    };

    let message_id = server
        .send_file_message_via_api_base(
            &target,
            file_path.to_str().expect("utf-8 file path"),
            None,
            None,
            &api_base,
        )
        .await
        .expect("feishu file send should succeed");
    assert_eq!(message_id, "om_file_123");

    let requests = mock.received_requests().await.expect("received requests");
    assert!(
        requests
            .iter()
            .any(|req| req.url.path() == "/open-apis/im/v1/files")
    );
    assert!(
        requests
            .iter()
            .any(|req| req.url.path() == "/open-apis/im/v1/messages")
    );

    let _ = tokio::fs::remove_file(&file_path).await;
}

// -- auth helper --

#[test]
fn test_require_auth_no_tokens() {
    let state = crate::server::create_app_state(GaryxConfig::default());
    let ctx = RunContext::default();
    assert!(GaryMcpServer::require_auth(&state, &ctx, None).is_ok());
}

#[test]
fn test_require_auth_with_tokens() {
    let state = crate::server::create_app_state(GaryxConfig::default());
    let mut state = (*state).clone_for_test();
    state.ops.restart_tokens = vec!["secret-1".to_owned()];
    let state = Arc::new(state);
    let ctx = RunContext::default();
    assert!(GaryMcpServer::require_auth(&state, &ctx, None).is_err());
    assert!(GaryMcpServer::require_auth(&state, &ctx, Some("wrong")).is_err());
    assert!(GaryMcpServer::require_auth(&state, &ctx, Some("secret-1")).is_ok());
}

#[test]
fn test_require_auth_from_header() {
    let state = crate::server::create_app_state(GaryxConfig::default());
    let mut state = (*state).clone_for_test();
    state.ops.restart_tokens = vec!["secret-1".to_owned()];
    let state = Arc::new(state);
    let ctx = RunContext {
        auth_token: Some("secret-1".to_owned()),
        ..Default::default()
    };
    assert!(GaryMcpServer::require_auth(&state, &ctx, None).is_ok());
}

// -- server info --

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
