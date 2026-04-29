use super::*;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::to_bytes;
use axum::extract::State;
use axum::response::IntoResponse;
use garyx_channels::{
    ChannelDispatcher, ChannelDispatcherImpl, ChannelInfo, FeishuSender, OutboundMessage,
};
use garyx_models::config::{ApiAccount, AutomationScheduleView, CronJobConfig, GaryxConfig};
use garyx_models::provider::ProviderMessage;
use garyx_models::routing::DeliveryContext;
use garyx_router::{
    ConversationIndexManager, InMemoryThreadStore, ThreadHistoryRepository, ThreadStore,
    ThreadTranscriptStore,
};
use tempfile::tempdir;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::cron::CronService;

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

async fn test_server_with_cron_service() -> GaryMcpServer {
    let state = crate::server::create_app_state(GaryxConfig::default());
    let data_dir = std::env::temp_dir().join(format!("garyx-mcp-cron-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(data_dir.join("cron").join("jobs"))
        .await
        .unwrap();

    let svc = Arc::new(CronService::new(data_dir));
    let mut state = (*state).clone_for_test();
    state.ops.cron_service = Some(svc);
    let state = Arc::new(state);
    GaryMcpServer::new(state)
}

async fn test_server_with_cron_job() -> GaryMcpServer {
    let server = test_server_with_cron_service().await;
    let svc = server
        .app_state
        .ops
        .cron_service
        .as_ref()
        .expect("cron service")
        .clone();
    svc.add(CronJobConfig {
        id: "job1".to_owned(),
        kind: Default::default(),
        label: None,
        schedule: CronSchedule::Interval { interval_secs: 60 },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
    })
    .await
    .unwrap();
    server
}

async fn test_server_with_automation_job() -> GaryMcpServer {
    let server = test_server_with_cron_service().await;
    let svc = server
        .app_state
        .ops
        .cron_service
        .as_ref()
        .expect("cron service")
        .clone();
    let cfg = crate::automation::build_automation_job(
        "job1",
        "Daily repo summary",
        "Summarize the latest repo activity.",
        "codex",
        "/tmp/gary-repo",
        AutomationScheduleView::Interval { hours: 6 },
        true,
    )
    .expect("automation job config");
    svc.add(cfg).await.unwrap();
    server
}

async fn automation_list_payload(state: Arc<crate::server::AppState>) -> Value {
    let response = crate::automation::list_automations(State(state))
        .await
        .into_response();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
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
        .await;
    state
        .threads
        .thread_store
        .set("s2::dm::u2", json!({}))
        .await;
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
        .await;
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
        .await;
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
            auto_research_role: None,
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
async fn test_auto_research_verdict_payload_stores_verdict_for_verify_thread() {
    let server = test_server();
    let payload = tools::auto_research::verdict_payload(
        &server,
        RunContext {
            thread_id: Some("thread::auto-research::ar_test::verify::1".to_owned()),
            auto_research_role: Some("verifier".to_owned()),
            ..Default::default()
        },
        AutoResearchVerdictParams {
            score: 8.8,
            feedback: "Grounded, all constraints satisfied. Ship it.".to_owned(),
        },
    )
    .await
    .expect("verdict payload should succeed");

    assert_eq!(payload["status"], "ok");
    let stored = server
        .app_state
        .ops
        .auto_research
        .take_verifier_verdict("thread::auto-research::ar_test::verify::1")
        .await
        .expect("stored verdict");
    assert_eq!(stored.score, 8.8);
    assert!(stored.feedback.contains("Grounded"));
}

#[tokio::test]
async fn test_auto_research_verdict_payload_rejects_missing_verifier_role() {
    let server = test_server();
    let error = tools::auto_research::verdict_payload(
        &server,
        RunContext {
            thread_id: Some("thread::auto-research::ar_test::verify::1".to_owned()),
            ..Default::default()
        },
        AutoResearchVerdictParams {
            score: 8.8,
            feedback: "Looks good".to_owned(),
        },
    )
    .await
    .unwrap_err();

    assert!(error.contains("only available"));
}

#[tokio::test]
async fn test_auto_research_verdict_payload_accepts_authorized_verify_thread_without_role_header() {
    let server = test_server();
    server
        .app_state
        .ops
        .auto_research
        .authorize_verifier_thread("thread::auto-research::ar_test::verify::1")
        .await;

    let payload = tools::auto_research::verdict_payload(
        &server,
        RunContext {
            thread_id: Some("thread::auto-research::ar_test::verify::1".to_owned()),
            ..Default::default()
        },
        AutoResearchVerdictParams {
            score: 9.1,
            feedback: "Auth fallback works. Keep tool path.".to_owned(),
        },
    )
    .await
    .expect("authorized verify thread should be accepted");

    assert_eq!(payload["status"], "ok");
    let stored = server
        .app_state
        .ops
        .auto_research
        .take_verifier_verdict("thread::auto-research::ar_test::verify::1")
        .await
        .expect("stored verdict");
    assert_eq!(stored.score, 9.1);
}

#[tokio::test]
async fn test_mcp_tool_metrics_record_success_and_error() {
    let server = test_server();

    let _ = server
        .status_payload(RunContext::default())
        .await
        .expect("status should succeed");
    server.record_tool_metric("status", "ok", std::time::Duration::from_millis(1));
    let bad_cron = CronParams {
        action: "invalid".to_owned(),
        job_id: None,
        schedule: None,
        schedule_view: None,
        interval_secs: None,
        at: None,
        job_action: None,
        cron_action: None,
        enabled: None,
        target: None,
        message: None,
        prompt: None,
        agent_id: None,
        label: None,
        workspace_dir: None,
        delete_after_run: None,
    };
    let _ = server
        .cron(Parameters(bad_cron))
        .await
        .expect_err("cron should fail");

    let metrics = server.app_state.ops.mcp_tool_metrics.snapshot();
    let status_ok = metrics
        .mcp_tool_calls_total
        .iter()
        .find(|m| m.tool == "status" && m.status == "ok")
        .map(|m| m.value)
        .unwrap_or(0);
    let cron_error = metrics
        .mcp_tool_calls_total
        .iter()
        .find(|m| m.tool == "cron" && m.status == "error")
        .map(|m| m.value)
        .unwrap_or(0);
    assert!(status_ok >= 1, "expected status ok call metric");
    assert!(cron_error >= 1, "expected cron error call metric");

    let status_duration = metrics
        .mcp_tool_duration_ms
        .iter()
        .find(|m| m.tool == "status")
        .map(|m| m.count)
        .unwrap_or(0);
    let cron_duration = metrics
        .mcp_tool_duration_ms
        .iter()
        .find(|m| m.tool == "cron")
        .map(|m| m.count)
        .unwrap_or(0);
    assert!(status_duration >= 1, "expected status duration metric");
    assert!(cron_duration >= 1, "expected cron duration metric");
}

// -- cron --

#[tokio::test]
async fn test_cron_list_ok() {
    let server = test_server();
    let params = CronParams {
        action: "list".to_owned(),
        job_id: None,
        schedule: None,
        schedule_view: None,
        interval_secs: None,
        at: None,
        job_action: None,
        cron_action: None,
        enabled: None,
        target: None,
        message: None,
        prompt: None,
        agent_id: None,
        label: None,
        workspace_dir: None,
        delete_after_run: None,
    };
    let result = server.cron(Parameters(params)).await;
    assert!(result.is_ok());
    let v: Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert_eq!(v["tool"], "cron");
    assert_eq!(v["action"], "list");
}

#[tokio::test]
async fn test_cron_invalid_action() {
    let server = test_server();
    let params = CronParams {
        action: "invalid".to_owned(),
        job_id: None,
        schedule: None,
        schedule_view: None,
        interval_secs: None,
        at: None,
        job_action: None,
        cron_action: None,
        enabled: None,
        target: None,
        message: None,
        prompt: None,
        agent_id: None,
        label: None,
        workspace_dir: None,
        delete_after_run: None,
    };
    let result = server.cron(Parameters(params)).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("invalid cron action"));
}

#[test]
fn test_parse_cron_action_aliases() {
    fn params_with_action(action: &str) -> CronParams {
        CronParams {
            action: "add".to_owned(),
            job_id: Some("j1".to_owned()),
            schedule: Some(json!("interval:60")),
            schedule_view: None,
            interval_secs: None,
            at: None,
            job_action: Some(action.to_owned()),
            cron_action: None,
            enabled: None,
            target: None,
            message: None,
            prompt: None,
            agent_id: None,
            label: None,
            workspace_dir: None,
            delete_after_run: None,
        }
    }

    assert_eq!(
        GaryMcpServer::parse_cron_action(&params_with_action("system_event")).unwrap(),
        CronAction::SystemEvent
    );
    assert_eq!(
        GaryMcpServer::parse_cron_action(&params_with_action("systemEvent")).unwrap(),
        CronAction::SystemEvent
    );
    assert_eq!(
        GaryMcpServer::parse_cron_action(&params_with_action("agent_turn")).unwrap(),
        CronAction::AgentTurn
    );
    assert_eq!(
        GaryMcpServer::parse_cron_action(&params_with_action("agentTurn")).unwrap(),
        CronAction::AgentTurn
    );
}

#[test]
fn test_cron_params_camel_case_aliases() {
    let parsed: CronParams = serde_json::from_value(json!({
        "action": "add",
        "jobId": "j1",
        "intervalSecs": 30,
        "jobAction": "agent_turn",
        "deleteAfterRun": true
    }))
    .unwrap();

    assert_eq!(parsed.job_id.as_deref(), Some("j1"));
    assert_eq!(parsed.interval_secs, Some(30));
    assert_eq!(parsed.job_action.as_deref(), Some("agent_turn"));
    assert_eq!(parsed.delete_after_run, Some(true));
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

#[test]
fn test_rebind_current_channel_params_camel_case_aliases() {
    let parsed: RebindCurrentChannelParams = serde_json::from_value(json!({
        "agentId": "reviewer",
        "workspaceDir": "/tmp/rebind"
    }))
    .unwrap();

    assert_eq!(parsed.agent_id, "reviewer");
    assert_eq!(parsed.workspace_dir, "/tmp/rebind");
}

#[test]
fn test_image_gen_params_camel_case_aliases() {
    let parsed: ImageGenParams = serde_json::from_value(json!({
        "prompt": "cat",
        "aspectRatio": "16:9",
        "imageSize": "2K",
        "referenceImages": ["/tmp/a.png"]
    }))
    .unwrap();

    assert_eq!(parsed.aspect_ratio.as_deref(), Some("16:9"));
    assert_eq!(parsed.image_size.as_deref(), Some("2K"));
    assert_eq!(
        parsed
            .reference_images
            .as_ref()
            .and_then(|v| v.first())
            .map(String::as_str),
        Some("/tmp/a.png")
    );
}

#[test]
fn test_conversation_history_params_camel_case_aliases() {
    let parsed: ConversationHistoryParams = serde_json::from_value(json!({
        "threadId": "thread::demo",
        "workspaceDir": "/tmp/demo",
        "from": "2026-03-20T10:00:00Z",
        "to": "2026-03-21",
        "limit": 20
    }))
    .unwrap();

    assert_eq!(parsed.thread_id.as_deref(), Some("thread::demo"));
    assert_eq!(parsed.workspace_dir.as_deref(), Some("/tmp/demo"));
    assert_eq!(parsed.from.as_deref(), Some("2026-03-20T10:00:00Z"));
    assert_eq!(parsed.to.as_deref(), Some("2026-03-21"));
    assert_eq!(parsed.limit, Some(20));
}

#[test]
fn test_conversation_search_params_camel_case_aliases() {
    let parsed: ConversationSearchParams = serde_json::from_value(json!({
        "query": "once protocol",
        "threadId": "thread::demo",
        "workspaceDir": "/tmp/demo",
        "from": "2026-03-20T10:00:00Z",
        "to": "2026-03-21",
        "limit": 3
    }))
    .unwrap();

    assert_eq!(parsed.query, "once protocol");
    assert_eq!(parsed.thread_id.as_deref(), Some("thread::demo"));
    assert_eq!(parsed.workspace_dir.as_deref(), Some("/tmp/demo"));
    assert_eq!(parsed.from.as_deref(), Some("2026-03-20T10:00:00Z"));
    assert_eq!(parsed.to.as_deref(), Some("2026-03-21"));
    assert_eq!(parsed.limit, Some(3));
}

#[tokio::test]
async fn test_rebind_current_channel_creates_thread_and_moves_current_binding() {
    let server = test_server();
    let state = server.app_state.clone();
    state
        .threads
        .thread_store
        .set(
            "thread::current",
            json!({
                "thread_id": "thread::current",
                "label": "Current",
                "channel_bindings": [
                    {
                        "channel": "telegram",
                        "account_id": "main",
                        "peer_id": "42",
                        "chat_id": "42",
                        "delivery_target_type": "chat_id",
                        "delivery_target_id": "42",
                        "display_label": "Alice"
                    }
                ]
            }),
        )
        .await;

    let payload = tools::rebind_current_channel::payload(
        &server,
        RunContext {
            thread_id: Some("thread::current".to_owned()),
            ..Default::default()
        },
        RebindCurrentChannelParams {
            agent_id: "claude".to_owned(),
            workspace_dir: "/tmp/rebind-workspace".to_owned(),
        },
    )
    .await
    .expect("rebind_current_channel should succeed");

    assert_eq!(payload["tool"], "rebind_current_channel");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["current_thread_id"], "thread::current");
    assert_eq!(payload["previous_thread_id"], "thread::current");
    assert_eq!(payload["requested_agent_id"], "claude");
    assert_eq!(payload["workspace_dir"], "/tmp/rebind-workspace");

    let new_thread_id = payload["thread_id"].as_str().expect("new thread id");
    assert_ne!(new_thread_id, "thread::current");
    let new_thread = state
        .threads
        .thread_store
        .get(new_thread_id)
        .await
        .expect("new thread stored");
    assert_eq!(new_thread["agent_id"], "claude");
    assert_eq!(new_thread["workspace_dir"], "/tmp/rebind-workspace");

    let old_thread = state
        .threads
        .thread_store
        .get("thread::current")
        .await
        .expect("old thread stored");
    let old_bindings = old_thread["channel_bindings"]
        .as_array()
        .expect("old bindings array");
    assert!(old_bindings.is_empty());

    let new_bindings = new_thread["channel_bindings"]
        .as_array()
        .expect("new bindings array");
    assert_eq!(new_bindings.len(), 1);
    assert_eq!(new_bindings[0]["channel"], "telegram");
    assert_eq!(new_bindings[0]["account_id"], "main");
    assert_eq!(new_bindings[0]["binding_key"], "42");

    let mut router = state.threads.router.lock().await;
    let resolved = router
        .resolve_endpoint_thread_id("telegram", "main", "42")
        .await;
    assert_eq!(resolved.as_deref(), Some(new_thread_id));
}

#[tokio::test]
async fn test_rebind_current_channel_uses_thread_origin_fields_to_pick_current_binding() {
    let server = test_server();
    let state = server.app_state.clone();
    state
        .threads
        .thread_store
        .set(
            "thread::current",
            json!({
                "thread_id": "thread::current",
                "label": "Current",
                "channel": "telegram",
                "account_id": "main",
                "from_id": "42",
                "channel_bindings": [
                    {
                        "channel": "telegram",
                        "account_id": "main",
                        "peer_id": "42",
                        "chat_id": "42",
                        "delivery_target_type": "chat_id",
                        "delivery_target_id": "42",
                        "display_label": "Alice"
                    },
                    {
                        "channel": "feishu",
                        "account_id": "ops",
                        "peer_id": "ou_other",
                        "chat_id": "oc_other",
                        "delivery_target_type": "chat_id",
                        "delivery_target_id": "oc_other",
                        "display_label": "Ops"
                    }
                ]
            }),
        )
        .await;

    let payload = tools::rebind_current_channel::payload(
        &server,
        RunContext {
            thread_id: Some("thread::current".to_owned()),
            ..Default::default()
        },
        RebindCurrentChannelParams {
            agent_id: "claude".to_owned(),
            workspace_dir: "/tmp/rebind-workspace".to_owned(),
        },
    )
    .await
    .expect("thread origin fields should disambiguate current binding");

    assert_eq!(payload["channel"], "telegram");
    assert_eq!(payload["account_id"], "main");
    assert_eq!(payload["from_id"], "42");
}

#[tokio::test]
async fn test_rebind_current_channel_errors_when_thread_binding_is_ambiguous() {
    let server = test_server();
    let state = server.app_state.clone();
    state
        .threads
        .thread_store
        .set(
            "thread::current",
            json!({
                "thread_id": "thread::current",
                "label": "Current",
                "channel_bindings": [
                    {
                        "channel": "telegram",
                        "account_id": "main",
                        "peer_id": "42",
                        "chat_id": "42",
                        "delivery_target_type": "chat_id",
                        "delivery_target_id": "42",
                        "display_label": "Alice"
                    },
                    {
                        "channel": "feishu",
                        "account_id": "ops",
                        "peer_id": "ou_other",
                        "chat_id": "oc_other",
                        "delivery_target_type": "chat_id",
                        "delivery_target_id": "oc_other",
                        "display_label": "Ops"
                    }
                ]
            }),
        )
        .await;

    let error = tools::rebind_current_channel::payload(
        &server,
        RunContext {
            thread_id: Some("thread::current".to_owned()),
            ..Default::default()
        },
        RebindCurrentChannelParams {
            agent_id: "claude".to_owned(),
            workspace_dir: "/tmp/rebind-workspace".to_owned(),
        },
    )
    .await
    .unwrap_err();

    assert!(error.contains("multiple channel bindings"));
}

#[tokio::test]
async fn test_conversation_history_formats_text_only_transcript() {
    let server = test_server();
    let state = server.app_state.clone();
    let thread_id = "thread::history-a";
    state
        .threads
        .thread_store
        .set(thread_id, json!({ "workspace_dir": "/tmp/workspace-a" }))
        .await;
    state
        .threads
        .history
        .transcript_store()
        .append_committed_messages(
            thread_id,
            None,
            &[
                ProviderMessage::user_text("hello\nthere")
                    .with_timestamp("2026-03-20T10:00:00Z")
                    .to_json_value(),
                json!({
                    "role": "assistant",
                    "content": "tool trace",
                    "tool_name": "shell",
                    "timestamp": "2026-03-20T10:00:01Z"
                }),
                ProviderMessage::assistant_text("hi back")
                    .with_timestamp("2026-03-20T10:00:02Z")
                    .to_json_value(),
            ],
        )
        .await
        .unwrap();

    let result = server
        .conversation_history(Parameters(ConversationHistoryParams {
            thread_id: Some(thread_id.to_owned()),
            workspace_dir: None,
            from: None,
            to: None,
            limit: None,
        }))
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(value["matched_threads"], 1);
    assert_eq!(value["matched_messages"], 2);
    assert_eq!(
        value["transcript"],
        Value::String("user: hello there\nassistant: hi back".to_owned())
    );
}

#[tokio::test]
async fn test_conversation_history_filters_by_workspace_and_time() {
    let server = test_server();
    let state = server.app_state.clone();

    state
        .threads
        .thread_store
        .set(
            "thread::history-ws-a-1",
            json!({ "workspace_dir": "/tmp/workspace-a" }),
        )
        .await;
    state
        .threads
        .history
        .transcript_store()
        .append_committed_messages(
            "thread::history-ws-a-1",
            None,
            &[
                ProviderMessage::user_text("old")
                    .with_timestamp("2026-03-20T09:00:00Z")
                    .to_json_value(),
                ProviderMessage::assistant_text("old reply")
                    .with_timestamp("2026-03-20T09:01:00Z")
                    .to_json_value(),
            ],
        )
        .await
        .unwrap();

    state
        .threads
        .thread_store
        .set(
            "thread::history-ws-a-2",
            json!({ "workspace_dir": "/tmp/workspace-a" }),
        )
        .await;
    state
        .threads
        .history
        .transcript_store()
        .append_committed_messages(
            "thread::history-ws-a-2",
            None,
            &[
                ProviderMessage::user_text("recent question")
                    .with_timestamp("2026-03-20T11:00:00Z")
                    .to_json_value(),
                ProviderMessage::assistant_text("recent answer")
                    .with_timestamp("2026-03-20T11:05:00Z")
                    .to_json_value(),
            ],
        )
        .await
        .unwrap();

    state
        .threads
        .thread_store
        .set(
            "thread::history-ws-b",
            json!({ "workspace_dir": "/tmp/workspace-b" }),
        )
        .await;
    state
        .threads
        .history
        .transcript_store()
        .append_committed_messages(
            "thread::history-ws-b",
            None,
            &[ProviderMessage::user_text("other workspace")
                .with_timestamp("2026-03-20T11:10:00Z")
                .to_json_value()],
        )
        .await
        .unwrap();

    let result = server
        .conversation_history(Parameters(ConversationHistoryParams {
            thread_id: None,
            workspace_dir: Some("/tmp/workspace-a".to_owned()),
            from: Some("2026-03-20T10:30:00Z".to_owned()),
            to: Some("2026-03-20T12:00:00Z".to_owned()),
            limit: None,
        }))
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&result).unwrap();

    assert_eq!(value["matched_threads"], 1);
    assert_eq!(value["matched_messages"], 2);
    assert_eq!(
        value["transcript"],
        Value::String("user: recent question\nassistant: recent answer".to_owned())
    );
}

#[tokio::test]
async fn test_conversation_search_returns_relevant_transcript_snippet() {
    let server = test_server();
    let state = server.app_state.clone();

    state
        .threads
        .thread_store
        .set(
            "thread::search-relevant",
            json!({ "workspace_dir": "/tmp/workspace-a" }),
        )
        .await;
    state
        .threads
        .history
        .transcript_store()
        .append_committed_messages(
            "thread::search-relevant",
            None,
            &[
                ProviderMessage::user_text("Can we support once schedule protocol?")
                    .with_timestamp("2026-03-20T11:00:00Z")
                    .to_json_value(),
                ProviderMessage::assistant_text("Yes, the format is ONCE:1992-10-03 11:11.")
                    .with_timestamp("2026-03-20T11:00:05Z")
                    .to_json_value(),
            ],
        )
        .await
        .unwrap();

    state
        .threads
        .thread_store
        .set(
            "thread::search-irrelevant",
            json!({ "workspace_dir": "/tmp/workspace-b" }),
        )
        .await;
    state
        .threads
        .history
        .transcript_store()
        .append_committed_messages(
            "thread::search-irrelevant",
            None,
            &[
                ProviderMessage::assistant_text("We also changed the settings layout.")
                    .with_timestamp("2026-03-20T11:01:00Z")
                    .to_json_value(),
            ],
        )
        .await
        .unwrap();

    let result = server
        .conversation_search(Parameters(ConversationSearchParams {
            query: "once schedule protocol".to_owned(),
            thread_id: None,
            workspace_dir: None,
            from: None,
            to: None,
            limit: Some(3),
        }))
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&result).unwrap();
    let results = value["results"].as_array().unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0]["thread_id"],
        Value::String("thread::search-relevant".to_owned())
    );
    assert_eq!(
        results[0]["workspace_dir"],
        Value::String("/tmp/workspace-a".to_owned())
    );
    assert!(
        results[0]["snippet"]
            .as_str()
            .unwrap()
            .contains("assistant: Yes, the format is ONCE:1992-10-03 11:11.")
    );
}

#[tokio::test]
async fn test_conversation_search_filters_by_workspace_and_time() {
    let server = test_server();
    let state = server.app_state.clone();

    state
        .threads
        .thread_store
        .set(
            "thread::search-old",
            json!({ "workspace_dir": "/tmp/workspace-a" }),
        )
        .await;
    state
        .threads
        .history
        .transcript_store()
        .append_committed_messages(
            "thread::search-old",
            None,
            &[ProviderMessage::assistant_text("Old automation thread.")
                .with_timestamp("2026-03-20T09:00:00Z")
                .to_json_value()],
        )
        .await
        .unwrap();

    state
        .threads
        .thread_store
        .set(
            "thread::search-recent",
            json!({ "workspace_dir": "/tmp/workspace-a" }),
        )
        .await;
    state
        .threads
        .history
        .transcript_store()
        .append_committed_messages(
            "thread::search-recent",
            None,
            &[
                ProviderMessage::user_text("Please remind me about automation smoke.")
                    .with_timestamp("2026-03-20T11:00:00Z")
                    .to_json_value(),
                ProviderMessage::assistant_text("Automation smoke covers once creation now.")
                    .with_timestamp("2026-03-20T11:02:00Z")
                    .to_json_value(),
            ],
        )
        .await
        .unwrap();

    state
        .threads
        .thread_store
        .set(
            "thread::search-other-workspace",
            json!({ "workspace_dir": "/tmp/workspace-b" }),
        )
        .await;
    state
        .threads
        .history
        .transcript_store()
        .append_committed_messages(
            "thread::search-other-workspace",
            None,
            &[
                ProviderMessage::assistant_text("Workspace B automation discussion.")
                    .with_timestamp("2026-03-20T11:05:00Z")
                    .to_json_value(),
            ],
        )
        .await
        .unwrap();

    let result = server
        .conversation_search(Parameters(ConversationSearchParams {
            query: "automation smoke once".to_owned(),
            thread_id: None,
            workspace_dir: Some("/tmp/workspace-a".to_owned()),
            from: Some("2026-03-20T10:30:00Z".to_owned()),
            to: Some("2026-03-20T12:00:00Z".to_owned()),
            limit: Some(5),
        }))
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&result).unwrap();
    let results = value["results"].as_array().unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0]["thread_id"],
        Value::String("thread::search-recent".to_owned())
    );
    assert_eq!(
        results[0]["snippet"],
        Value::String(
            "user: Please remind me about automation smoke.\nassistant: Automation smoke covers once creation now."
                .to_owned()
        )
    );
}

#[tokio::test]
async fn test_conversation_search_uses_vector_index_backend_and_returns_metadata() {
    let thread_store = Arc::new(InMemoryThreadStore::new());
    thread_store
        .set(
            "thread::vector-search",
            json!({ "workspace_dir": "/tmp/workspace-a" }),
        )
        .await;

    let temp = tempdir().unwrap();
    let transcript_store = Arc::new(
        ThreadTranscriptStore::file(temp.path().join("transcripts"))
            .await
            .unwrap(),
    );
    transcript_store
        .append_committed_messages(
            "thread::vector-search",
            None,
            &[
                ProviderMessage::user_text("Please remember the once schedule protocol.")
                    .with_timestamp("2026-03-20T11:00:00Z")
                    .to_json_value(),
                ProviderMessage::assistant_text("Use ONCE:1992-10-03 11:11.")
                    .with_timestamp("2026-03-20T11:00:05Z")
                    .to_json_value(),
            ],
        )
        .await
        .unwrap();

    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                {
                    "embedding": [1.0, 0.0, 0.0]
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let conversation_index = ConversationIndexManager::new(
        thread_store.clone(),
        transcript_store.clone(),
        temp.path().join("conversation-index").join("index.sqlite3"),
        garyx_models::config::ConversationIndexConfig {
            enabled: true,
            api_key: "test-key".to_owned(),
            model: "text-embedding-3-small".to_owned(),
            base_url: format!("{}/v1", mock_server.uri()),
        },
    )
    .await
    .unwrap();

    let thread_history = Arc::new(
        ThreadHistoryRepository::new(thread_store.clone(), transcript_store)
            .with_conversation_index(conversation_index.clone()),
    );
    conversation_index.enqueue_thread("thread::vector-search");

    let state = crate::server::AppStateBuilder::new(GaryxConfig::default())
        .with_thread_store(thread_store)
        .with_thread_history(thread_history)
        .build();
    let server = GaryMcpServer::new(state);

    let value = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let result = server
                .conversation_search(Parameters(ConversationSearchParams {
                    query: "once schedule protocol".to_owned(),
                    thread_id: None,
                    workspace_dir: None,
                    from: None,
                    to: None,
                    limit: Some(3),
                }))
                .await
                .unwrap();
            let value: Value = serde_json::from_str(&result).unwrap();
            if value["backend"] == "vector_index"
                && value["results"]
                    .as_array()
                    .is_some_and(|results| !results.is_empty())
            {
                return value;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();

    let results = value["results"].as_array().unwrap();
    assert_eq!(value["backend"], "vector_index");
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0]["thread_id"],
        Value::String("thread::vector-search".to_owned())
    );
    assert_eq!(
        results[0]["workspace_dir"],
        Value::String("/tmp/workspace-a".to_owned())
    );
    assert!(
        results[0]["transcript_file"]
            .as_str()
            .unwrap_or_default()
            .ends_with(".jsonl")
    );
    assert!(
        results[0]["snippet"]
            .as_str()
            .unwrap()
            .contains("assistant: Use ONCE:1992-10-03 11:11.")
    );
}

#[tokio::test]
async fn test_cron_status_update_and_run_alias() {
    let server = test_server_with_automation_job().await;

    let status = server
        .cron(Parameters(CronParams {
            action: "status".to_owned(),
            job_id: Some("job1".to_owned()),
            schedule: None,
            schedule_view: None,
            interval_secs: None,
            at: None,
            job_action: None,
            cron_action: None,
            enabled: None,
            target: None,
            message: None,
            prompt: None,
            agent_id: None,
            label: None,
            workspace_dir: None,
            delete_after_run: None,
        }))
        .await
        .unwrap();
    let status_v: Value = serde_json::from_str(&status).unwrap();
    assert_eq!(status_v["job"]["kind"], "automation_prompt");
    assert_eq!(status_v["job"]["job_action"], "agent_turn");
    assert_eq!(status_v["job"]["schedule_view"]["kind"], "interval");
    assert_eq!(status_v["job"]["schedule_view"]["hours"], 6);
    assert_eq!(
        status_v["job"]["workspace_dir"],
        Value::String("/tmp/gary-repo".to_owned())
    );

    let update = server
        .cron(Parameters(CronParams {
            action: "update".to_owned(),
            job_id: Some("job1".to_owned()),
            schedule: Some(json!({
                "kind": "interval",
                "hours": 12
            })),
            schedule_view: None,
            interval_secs: None,
            at: None,
            job_action: None,
            cron_action: None,
            enabled: Some(false),
            target: None,
            message: Some("Ping the repo and summarize any changes.".to_owned()),
            prompt: None,
            agent_id: None,
            label: Some("Repo sweep".to_owned()),
            workspace_dir: Some("/tmp/gary-repo-next".to_owned()),
            delete_after_run: None,
        }))
        .await
        .unwrap();
    let update_v: Value = serde_json::from_str(&update).unwrap();
    assert_eq!(update_v["action"], "update");
    assert_eq!(update_v["job"]["kind"], "automation_prompt");
    assert_eq!(update_v["job"]["job_action"], "agent_turn");
    assert_eq!(update_v["job"]["enabled"], false);
    assert_eq!(update_v["job"]["label"], "Repo sweep");
    assert_eq!(
        update_v["job"]["message"],
        "Ping the repo and summarize any changes."
    );
    assert_eq!(update_v["job"]["workspace_dir"], "/tmp/gary-repo-next");
    assert_eq!(update_v["job"]["schedule_view"]["kind"], "interval");
    assert_eq!(update_v["job"]["schedule_view"]["hours"], 12);

    let run = server
        .cron(Parameters(CronParams {
            action: "run".to_owned(),
            job_id: Some("job1".to_owned()),
            schedule: None,
            schedule_view: None,
            interval_secs: None,
            at: None,
            job_action: None,
            cron_action: None,
            enabled: None,
            target: None,
            message: None,
            prompt: None,
            agent_id: None,
            label: None,
            workspace_dir: None,
            delete_after_run: None,
        }))
        .await;
    assert!(run.is_err(), "disabled job should not run");
    assert!(
        run.unwrap_err().contains("Cron job not found: job1"),
        "disabled run should mirror python not-found style response"
    );
}

#[tokio::test]
async fn test_cron_add_creates_automation_visible_in_automation_list() {
    let server = test_server_with_cron_service().await;
    let result = server
        .cron(Parameters(CronParams {
            action: "add".to_owned(),
            job_id: Some("daily-triage".to_owned()),
            schedule: Some(json!({
                "kind": "interval",
                "hours": 24
            })),
            schedule_view: None,
            interval_secs: None,
            at: None,
            job_action: None,
            cron_action: None,
            enabled: Some(true),
            target: None,
            message: Some("Review the repo and summarize open work.".to_owned()),
            prompt: None,
            agent_id: None,
            label: Some("Daily triage".to_owned()),
            workspace_dir: Some("/tmp/gary-repo".to_owned()),
            delete_after_run: None,
        }))
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(value["job"]["kind"], "automation_prompt");
    assert_eq!(value["job"]["label"], "Daily triage");
    assert_eq!(value["job"]["workspace_dir"], "/tmp/gary-repo");
    assert_eq!(value["job"]["schedule_view"]["kind"], "interval");
    assert_eq!(value["job"]["schedule_view"]["hours"], 24);

    let payload = automation_list_payload(server.app_state.clone()).await;
    let automations = payload["automations"].as_array().unwrap();
    assert_eq!(automations.len(), 1);
    assert_eq!(automations[0]["id"], "daily-triage");
    assert_eq!(automations[0]["label"], "Daily triage");
    assert_eq!(automations[0]["agentId"], "claude");
    assert_eq!(
        automations[0]["prompt"],
        "Review the repo and summarize open work."
    );
    assert_eq!(automations[0]["workspaceDir"], "/tmp/gary-repo");
    assert_eq!(automations[0]["schedule"]["kind"], "interval");
    assert_eq!(automations[0]["schedule"]["hours"], 24);
}

#[tokio::test]
async fn test_cron_add_infers_automation_schedule_from_interval_secs() {
    let server = test_server_with_cron_service().await;
    let result = server
        .cron(Parameters(CronParams {
            action: "add".to_owned(),
            job_id: Some("every-six-hours".to_owned()),
            schedule: None,
            schedule_view: None,
            interval_secs: Some(6 * 3600),
            at: None,
            job_action: Some("agent_turn".to_owned()),
            cron_action: None,
            enabled: Some(true),
            target: None,
            message: None,
            prompt: Some("Check the queue and summarize new failures.".to_owned()),
            agent_id: None,
            label: None,
            workspace_dir: Some("/tmp/queue".to_owned()),
            delete_after_run: None,
        }))
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(value["job"]["schedule_view"]["kind"], "interval");
    assert_eq!(value["job"]["schedule_view"]["hours"], 6);
}

#[tokio::test]
async fn test_cron_add_accepts_one_time_schedule_text() {
    let server = test_server_with_cron_service().await;
    let result = server
        .cron(Parameters(CronParams {
            action: "add".to_owned(),
            job_id: Some("once-triage".to_owned()),
            schedule: Some(json!("ONCE:2030-05-01 08:30")),
            schedule_view: None,
            interval_secs: None,
            at: None,
            job_action: Some("agent_turn".to_owned()),
            cron_action: None,
            enabled: Some(true),
            target: None,
            message: None,
            prompt: Some("Check the release checklist once.".to_owned()),
            agent_id: None,
            label: Some("One-time triage".to_owned()),
            workspace_dir: Some("/tmp/once-repo".to_owned()),
            delete_after_run: None,
        }))
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(value["job"]["schedule_view"]["kind"], "once");
    assert_eq!(value["job"]["schedule_view"]["at"], "2030-05-01T08:30");

    let payload = automation_list_payload(server.app_state.clone()).await;
    let automations = payload["automations"].as_array().unwrap();
    assert_eq!(automations.len(), 1);
    assert_eq!(automations[0]["schedule"]["kind"], "once");
    assert_eq!(automations[0]["schedule"]["at"], "2030-05-01T08:30");
}

#[tokio::test]
async fn test_cron_update_rewrites_non_automation_scheduler_job_as_automation() {
    let server = test_server_with_cron_job().await;
    let result = server
        .cron(Parameters(CronParams {
            action: "update".to_owned(),
            job_id: Some("job1".to_owned()),
            schedule: Some(json!({
                "kind": "interval",
                "hours": 12
            })),
            schedule_view: None,
            interval_secs: None,
            at: None,
            job_action: None,
            cron_action: None,
            enabled: Some(true),
            target: None,
            message: None,
            prompt: Some("Summarize the latest support issues.".to_owned()),
            agent_id: None,
            label: Some("Support triage".to_owned()),
            workspace_dir: Some("/tmp/support".to_owned()),
            delete_after_run: None,
        }))
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(value["job"]["kind"], "automation_prompt");
    assert_eq!(value["job"]["label"], "Support triage");
    assert_eq!(value["job"]["workspace_dir"], "/tmp/support");
    assert_eq!(value["job"]["schedule_view"]["hours"], 12);

    let payload = automation_list_payload(server.app_state.clone()).await;
    let automations = payload["automations"].as_array().unwrap();
    assert_eq!(automations.len(), 1);
    assert_eq!(automations[0]["id"], "job1");
    assert_eq!(automations[0]["label"], "Support triage");
    assert_eq!(automations[0]["workspaceDir"], "/tmp/support");
}

#[tokio::test]
async fn test_cron_add_rejects_legacy_non_automation_job_action() {
    let server = test_server_with_cron_service().await;
    let result = server
        .cron(Parameters(CronParams {
            action: "add".to_owned(),
            job_id: Some("log".to_owned()),
            schedule: None,
            schedule_view: None,
            interval_secs: Some(3600),
            at: None,
            job_action: Some("log".to_owned()),
            cron_action: None,
            enabled: Some(true),
            target: None,
            message: None,
            prompt: Some("ignored".to_owned()),
            agent_id: None,
            label: None,
            workspace_dir: Some("/tmp/repo".to_owned()),
            delete_after_run: None,
        }))
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unsupported job_action: log"),);
}

// -- image_gen --

#[tokio::test]
async fn test_image_gen_ok() {
    let server = test_server();
    let result = server
        .image_gen(Parameters(ImageGenParams {
            prompt: "a cat".to_owned(),
            size: None,
            aspect_ratio: None,
            image_size: None,
            reference_images: None,
        }))
        .await;
    assert!(result.is_ok());
    let v: Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert_eq!(v["tool"], "image_gen");
    assert_eq!(v["prompt"], "a cat");
}

#[tokio::test]
async fn test_image_gen_missing_key_reports_config_hint() {
    let server = test_server();
    let result = server
        .image_gen(Parameters(ImageGenParams {
            prompt: "a cat".to_owned(),
            size: None,
            aspect_ratio: Some("1:1".to_owned()),
            image_size: Some("2K".to_owned()),
            reference_images: None,
        }))
        .await
        .expect("image_gen should return wrapper response");

    let v: Value = serde_json::from_str(&result).expect("valid image_gen json");
    if GaryMcpServer::resolve_image_gen_api_key("").is_some() {
        let status = v["status"].as_str().unwrap_or_default();
        assert!(
            status == "ok" || status == "error",
            "unexpected image_gen status when API key is configured: {status}"
        );
    } else {
        assert_eq!(v["status"], "error");
        let err = v["result"]["error"].as_str().unwrap_or_default();
        assert!(
            err.contains("garyx.json") && err.contains("gateway.image_gen.api_key"),
            "expected config hint in missing-key error, got: {err}"
        );
    }
}

#[tokio::test]
#[ignore = "requires GEMINI_API_KEY/GOOGLE_API_KEY and network access"]
async fn test_image_gen_live_with_gemini() {
    let api_key = std::env::var("GEMINI_API_KEY")
        .ok()
        .or_else(|| std::env::var("GOOGLE_API_KEY").ok())
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .expect("missing GEMINI_API_KEY/GOOGLE_API_KEY");

    let mut cfg = GaryxConfig::default();
    cfg.gateway.image_gen.api_key = api_key;
    if let Ok(model) = std::env::var("GARYX_IMAGE_GEN_MODEL") {
        if !model.trim().is_empty() {
            cfg.gateway.image_gen.model = model;
        }
    }
    let server = GaryMcpServer::new(crate::server::create_app_state(cfg));
    let result = server
        .image_gen(Parameters(ImageGenParams {
            prompt: "minimal red circle on white background, flat icon style".to_owned(),
            size: None,
            aspect_ratio: Some("1:1".to_owned()),
            image_size: Some("2K".to_owned()),
            reference_images: None,
        }))
        .await
        .expect("image_gen tool should return success wrapper");

    let v: Value = serde_json::from_str(&result).expect("valid image_gen json");
    assert_eq!(v["tool"], "image_gen");
    assert_eq!(
        v["status"], "ok",
        "image_gen backend failed: {}",
        v["result"]
    );
    assert_eq!(v["result"]["success"], true);

    let image_path = v["result"]["image_path"]
        .as_str()
        .expect("image_gen should return output path");
    let meta = tokio::fs::metadata(image_path)
        .await
        .expect("generated image should exist");
    assert!(meta.len() > 0, "generated image file should not be empty");
}

#[tokio::test]
async fn test_image_gen_invalid_size() {
    let server = test_server();
    let result = server
        .image_gen(Parameters(ImageGenParams {
            prompt: "a cat".to_owned(),
            size: Some("999x999".to_owned()),
            aspect_ratio: None,
            image_size: None,
            reference_images: None,
        }))
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("invalid size"));
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
    assert_eq!(calls[0].text, "#cron::daily\nscheduled");
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
        .await;

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
    assert_eq!(calls[0].text, "#cron::daily\nscheduled");
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
    assert_eq!(calls[0].text, "hello current");
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
        .await;

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
        .await;

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
        .await;

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
    assert!(instructions.contains("rebind_current_channel"));
    assert!(!instructions.contains("speak_to_agent"));
    assert!(!instructions.contains("auto_research_verdict"));
    assert!(!instructions.contains("update_team_status"));
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
