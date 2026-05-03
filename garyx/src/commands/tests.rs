use super::*;
use axum::{
    Json, Router,
    extract::Path as AxumPath,
    http::StatusCode,
    routing::{get, post, put},
};
use garyx_router::file_store::thread_storage_file_name;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::sync::{Arc as StdArc, Mutex};
use tempfile::tempdir;
use tokio::{net::TcpListener, task::JoinHandle};

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: String,
    path: String,
    body: Value,
}

struct ScopedEnvVar {
    key: &'static str,
    previous: Option<OsString>,
}

impl ScopedEnvVar {
    fn set_path(key: &'static str, value: &Path) -> Self {
        Self::set_value(key, value.as_os_str())
    }

    fn set_string(key: &'static str, value: &str) -> Self {
        Self::set_value(key, OsStr::new(value))
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::remove_var(key);
        }
        Self { key, previous }
    }

    fn set_value(key: &'static str, value: &OsStr) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        unsafe {
            if let Some(value) = &self.previous {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}

fn write_test_plugin_bundle(root: &Path, plugin_id: &str, required_fields: &[&str]) -> PathBuf {
    let plugin_dir = root.join(plugin_id);
    std::fs::create_dir_all(&plugin_dir).expect("create plugin dir");
    let binary_name = if cfg!(windows) {
        "fake-plugin.cmd"
    } else {
        "fake-plugin.sh"
    };
    let binary_path = plugin_dir.join(binary_name);
    if cfg!(windows) {
        std::fs::write(&binary_path, "@echo off\r\nexit /b 0\r\n").expect("write fake plugin");
    } else {
        std::fs::write(&binary_path, "#!/bin/sh\nexit 0\n").expect("write fake plugin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&binary_path)
                .expect("fake plugin metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&binary_path, permissions).expect("chmod fake plugin");
        }
    }

    let required = serde_json::to_string(required_fields).expect("required fields json");
    let manifest = format!(
        r#"[plugin]
id = "{plugin_id}"
version = "0.1.0"
display_name = "Test {plugin_id}"

[entry]
binary = "./{binary_name}"

[capabilities]
delivery_model = "pull_explicit_ack"
outbound = true
inbound = true

[schema]
type = "object"
required = {required}

[schema.properties.token]
type = "string"

[schema.properties.base_url]
type = "string"
"#
    );
    std::fs::write(plugin_dir.join("plugin.toml"), manifest).expect("write plugin manifest");
    plugin_dir
}

fn write_empty_config_file(dir: &tempfile::TempDir) -> PathBuf {
    let config_path = dir.path().join("gary.json");
    std::fs::write(
        &config_path,
        serde_json::to_vec_pretty(&GaryxConfig::default()).expect("config json"),
    )
    .expect("write config");
    config_path
}

fn write_test_gateway_config(dir: &tempfile::TempDir, public_url: &str) -> std::path::PathBuf {
    let config_path = dir.path().join("gary.json");
    std::fs::write(
        &config_path,
        serde_json::to_vec_pretty(&json!({
            "gateway": {
                "public_url": public_url
            }
        }))
        .expect("config json"),
    )
    .expect("write config");
    config_path
}

async fn spawn_agent_http_test_server(
    requests: StdArc<Mutex<Vec<RecordedRequest>>>,
    put_status: StatusCode,
) -> (String, JoinHandle<()>) {
    let post_requests = requests.clone();
    let put_requests = requests.clone();
    let app = Router::new()
        .route(
            "/api/custom-agents",
            post(move |Json(payload): Json<Value>| {
                let requests = post_requests.clone();
                async move {
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "POST".to_owned(),
                            path: "/api/custom-agents".to_owned(),
                            body: payload.clone(),
                        });
                    (
                        StatusCode::CREATED,
                        Json(json!({
                            "agent_id": payload["agent_id"],
                            "display_name": payload["display_name"],
                            "provider_type": payload["provider_type"],
                            "model": payload["model"],
                            "system_prompt": payload["system_prompt"],
                            "built_in": false,
                        })),
                    )
                }
            }),
        )
        .route(
            "/api/custom-agents/{agent_id}",
            put(
                move |AxumPath(agent_id): AxumPath<String>, Json(payload): Json<Value>| {
                    let requests = put_requests.clone();
                    async move {
                        let path = format!("/api/custom-agents/{agent_id}");
                        requests
                            .lock()
                            .expect("request lock")
                            .push(RecordedRequest {
                                method: "PUT".to_owned(),
                                path,
                                body: payload.clone(),
                            });
                        if put_status.is_success() {
                            (
                                put_status,
                                Json(json!({
                                    "agent_id": agent_id,
                                    "display_name": payload["display_name"],
                                    "provider_type": payload["provider_type"],
                                    "model": payload["model"],
                                    "system_prompt": payload["system_prompt"],
                                    "built_in": false,
                                })),
                            )
                        } else {
                            (
                                put_status,
                                Json(json!({ "error": "custom agent not found" })),
                            )
                        }
                    }
                },
            ),
        );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("listener addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test router");
    });
    (format!("http://{addr}"), handle)
}

async fn spawn_automation_http_test_server(
    requests: StdArc<Mutex<Vec<RecordedRequest>>>,
) -> (String, JoinHandle<()>) {
    let list_requests = requests.clone();
    let create_requests = requests.clone();
    let get_requests = requests.clone();
    let update_requests = requests.clone();
    let delete_requests = requests.clone();
    let run_requests = requests.clone();
    let activity_requests = requests.clone();

    let app = Router::new()
        .route(
            "/api/automations",
            get(move || {
                let requests = list_requests.clone();
                async move {
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "GET".to_owned(),
                            path: "/api/automations".to_owned(),
                            body: Value::Null,
                        });
                    (
                        StatusCode::OK,
                        Json(json!({
                            "automations": []
                        })),
                    )
                }
            })
            .post(move |Json(payload): Json<Value>| {
                let requests = create_requests.clone();
                async move {
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "POST".to_owned(),
                            path: "/api/automations".to_owned(),
                            body: payload.clone(),
                        });
                    (
                        StatusCode::CREATED,
                        Json(json!({
                            "id": "automation::created",
                            "label": payload["label"],
                            "prompt": payload["prompt"],
                            "agentId": payload.get("agentId").cloned().unwrap_or_else(|| json!("claude")),
                            "enabled": payload.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                            "workspaceDir": payload["workspaceDir"],
                            "nextRun": "2030-05-01T08:30:00Z",
                            "lastStatus": "skipped",
                            "schedule": payload["schedule"],
                        })),
                    )
                }
            }),
        )
        .route(
            "/api/automations/{automation_id}",
            get(move |AxumPath(automation_id): AxumPath<String>| {
                let requests = get_requests.clone();
                async move {
                    let path = format!("/api/automations/{automation_id}");
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "GET".to_owned(),
                            path,
                            body: Value::Null,
                        });
                    (
                        StatusCode::OK,
                        Json(json!({
                            "id": automation_id,
                            "label": "Daily triage",
                            "prompt": "Summarize repo state",
                            "agentId": "claude",
                            "enabled": true,
                            "workspaceDir": "/tmp/repo",
                            "nextRun": "2030-05-01T08:30:00Z",
                            "lastStatus": "skipped",
                            "schedule": {"kind": "interval", "hours": 6},
                        })),
                    )
                }
            })
            .patch(
                move |AxumPath(automation_id): AxumPath<String>, Json(payload): Json<Value>| {
                    let requests = update_requests.clone();
                    async move {
                        let path = format!("/api/automations/{automation_id}");
                        requests
                            .lock()
                            .expect("request lock")
                            .push(RecordedRequest {
                                method: "PATCH".to_owned(),
                                path,
                                body: payload.clone(),
                            });
                        (
                            StatusCode::OK,
                            Json(json!({
                                "id": automation_id,
                                "label": payload.get("label").cloned().unwrap_or_else(|| json!("Daily triage")),
                                "prompt": payload.get("prompt").cloned().unwrap_or_else(|| json!("Summarize repo state")),
                                "agentId": payload.get("agentId").cloned().unwrap_or_else(|| json!("claude")),
                                "enabled": payload.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                                "workspaceDir": payload.get("workspaceDir").cloned().unwrap_or_else(|| json!("/tmp/repo")),
                                "nextRun": "2030-05-01T08:30:00Z",
                                "lastStatus": "skipped",
                                "schedule": payload.get("schedule").cloned().unwrap_or_else(|| json!({"kind": "interval", "hours": 6})),
                            })),
                        )
                    }
                },
            )
            .delete(move |AxumPath(automation_id): AxumPath<String>| {
                let requests = delete_requests.clone();
                async move {
                    let path = format!("/api/automations/{automation_id}");
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "DELETE".to_owned(),
                            path,
                            body: Value::Null,
                        });
                    (
                        StatusCode::OK,
                        Json(json!({
                            "deleted": true,
                            "id": automation_id,
                        })),
                    )
                }
            }),
        )
        .route(
            "/api/automations/{automation_id}/run-now",
            post(move |AxumPath(automation_id): AxumPath<String>| {
                let requests = run_requests.clone();
                async move {
                    let path = format!("/api/automations/{automation_id}/run-now");
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "POST".to_owned(),
                            path,
                            body: Value::Null,
                        });
                    (
                        StatusCode::OK,
                        Json(json!({
                            "runId": "run-1",
                            "status": "success",
                            "startedAt": "2030-05-01T08:30:00Z",
                            "finishedAt": "2030-05-01T08:30:01Z",
                            "durationMs": 1000,
                            "threadId": "thread::automation-test",
                        })),
                    )
                }
            }),
        )
        .route(
            "/api/automations/{automation_id}/activity",
            get(move |AxumPath(automation_id): AxumPath<String>| {
                let requests = activity_requests.clone();
                async move {
                    let path = format!("/api/automations/{automation_id}/activity");
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "GET".to_owned(),
                            path,
                            body: Value::Null,
                        });
                    (
                        StatusCode::OK,
                        Json(json!({
                            "items": [],
                            "threadId": null,
                            "count": 0,
                        })),
                    )
                }
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("listener addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test router");
    });
    (format!("http://{addr}"), handle)
}

#[tokio::test]
async fn migrate_thread_transcripts_rewrites_records_and_transcripts() {
    let data_dir = tempdir().expect("data dir");
    let backup_dir = tempdir().expect("backup dir");
    let store = FileThreadStore::new(data_dir.path())
        .await
        .expect("thread store");
    let store: Arc<dyn ThreadStore> = Arc::new(store);
    let thread_id = "thread::migrate-cli";
    store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "messages": [
                    {
                        "role": "user",
                        "content": "hello",
                        "run_id": "run-1",
                        "timestamp": "2026-03-20T10:00:00Z"
                    },
                    {
                        "role": "assistant",
                        "content": "world",
                        "run_id": "run-1",
                        "timestamp": "2026-03-20T10:00:01Z"
                    }
                ]
            }),
        )
        .await;

    cmd_migrate_thread_transcripts(
        "/tmp/unused-gary-config.json",
        Some(data_dir.path().to_str().expect("data dir str")),
        Some(backup_dir.path().to_str().expect("backup dir str")),
        true,
    )
    .await
    .expect("migration should succeed");

    let rewritten = store.get(thread_id).await.expect("rewritten record");
    assert_eq!(rewritten["message_count"], 2);
    assert_eq!(rewritten["messages"].as_array().map(Vec::len), Some(2));
    assert_eq!(rewritten["history"]["source"], "transcript_v1");
    assert_eq!(rewritten["history"]["message_count"], 2);
    assert_eq!(rewritten["history"]["recent_committed_run_ids"][0], "run-1");

    let transcript_dir = thread_transcripts_dir_for_data_dir(data_dir.path());
    let transcript_path = transcript_dir.join(thread_storage_file_name(thread_id, "jsonl"));
    assert!(
        transcript_path.exists(),
        "expected transcript file to be written"
    );

    let transcript_store = ThreadTranscriptStore::file(&transcript_dir)
        .await
        .expect("transcript store");
    assert_eq!(
        transcript_store
            .message_count(thread_id)
            .await
            .expect("message count"),
        2
    );
    let tail = transcript_store.tail(thread_id, 2).await.expect("tail");
    assert_eq!(tail[0]["content"], "hello");
    assert_eq!(tail[1]["content"], "world");

    let backup_path = backup_dir
        .path()
        .join(format!("{}.json", encode_thread_backup_key(thread_id)));
    assert!(backup_path.exists(), "expected original thread JSON backup");
}

#[test]
fn build_provider_metadata_only_for_local_gateway() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    unsafe {
        std::env::set_var(CLAUDE_OAUTH_ENV, "claude-token");
        std::env::set_var(CODEX_API_KEY_ENV, "codex-key");
    }

    let local = build_provider_metadata_for_local_gateway("http://127.0.0.1:31337")
        .expect("local gateway should inject metadata");
    assert_eq!(
        local[CLAUDE_ENV_METADATA_KEY][CLAUDE_OAUTH_ENV],
        "claude-token"
    );
    assert_eq!(
        local[CODEX_ENV_METADATA_KEY][CODEX_API_KEY_ENV],
        "codex-key"
    );
    assert!(
        build_provider_metadata_for_local_gateway("https://gary.example.com").is_none(),
        "remote gateway should not receive local auth metadata"
    );

    unsafe {
        std::env::remove_var(CLAUDE_OAUTH_ENV);
        std::env::remove_var(CODEX_API_KEY_ENV);
    }
}

#[test]
fn build_provider_metadata_omits_empty_values() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    unsafe {
        std::env::remove_var(CLAUDE_OAUTH_ENV);
        std::env::remove_var(CODEX_API_KEY_ENV);
    }
    assert!(build_provider_metadata_for_local_gateway("http://127.0.0.1:31337").is_none());
}

#[test]
fn cli_actor_header_uses_agent_identity_from_env() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let _actor = ScopedEnvVar::remove("GARYX_ACTOR");
    let _agent = ScopedEnvVar::set_string("GARYX_AGENT_ID", "codex");
    let _user = ScopedEnvVar::set_string("GARYX_USER", "owner");

    assert_eq!(cli_actor_header_value(), "agent:codex");
    assert_eq!(
        cli_actor_payload(),
        json!({ "kind": "agent", "agent_id": "codex" })
    );
}

#[test]
fn cli_actor_header_prefers_explicit_actor_env() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let _actor = ScopedEnvVar::set_string("GARYX_ACTOR", "human:alice");
    let _agent = ScopedEnvVar::set_string("GARYX_AGENT_ID", "codex");
    let _user = ScopedEnvVar::set_string("GARYX_USER", "owner");

    assert_eq!(cli_actor_header_value(), "human:alice");
    assert_eq!(
        cli_actor_payload(),
        json!({ "kind": "human", "user_id": "alice" })
    );
}

#[test]
fn gateway_base_url_prefers_public_url() {
    let dir = tempdir().expect("tempdir");
    let config_path = dir.path().join("gary.json");
    std::fs::write(
        &config_path,
        serde_json::to_vec_pretty(&json!({
            "gateway": {
                "host": "0.0.0.0",
                "port": 3000,
                "public_url": "http://127.0.0.1:31337"
            }
        }))
        .expect("config json"),
    )
    .expect("write config");
    let base_url = gateway_base_url(config_path.to_str().expect("config path")).expect("base url");
    assert_eq!(base_url, "http://127.0.0.1:31337");
}

#[tokio::test]
async fn cmd_command_set_get_and_delete_persist_shortcut() {
    let dir = tempdir().expect("tempdir");
    let config_path = dir.path().join("gary.json");
    std::fs::write(
        &config_path,
        serde_json::to_vec_pretty(&json!({
            "gateway": {
                "host": "127.0.0.1",
                "port": 9
            },
            "commands": []
        }))
        .expect("config json"),
    )
    .expect("write config");

    cmd_command_set(
        config_path.to_str().expect("config path"),
        "/summary".to_owned(),
        Some("Summarize the current thread".to_owned()),
        Some("Summarize thread".to_owned()),
        true,
    )
    .await
    .expect("set shortcut");
    cmd_command_get(config_path.to_str().expect("config path"), "summary", true)
        .expect("get shortcut");

    let loaded = load_config_or_default(
        config_path.to_str().expect("config path"),
        ConfigRuntimeOverrides::default(),
    )
    .expect("load config");
    assert_eq!(loaded.config.commands.len(), 1);
    assert_eq!(loaded.config.commands[0].name, "summary");
    assert_eq!(
        loaded.config.commands[0].prompt.as_deref(),
        Some("Summarize the current thread")
    );

    cmd_command_delete(config_path.to_str().expect("config path"), "/summary", true)
        .await
        .expect("delete shortcut");
    let loaded = load_config_or_default(
        config_path.to_str().expect("config path"),
        ConfigRuntimeOverrides::default(),
    )
    .expect("reload config");
    assert!(loaded.config.commands.is_empty());
}

#[tokio::test]
async fn cmd_command_set_rejects_builtin_collision() {
    let dir = tempdir().expect("tempdir");
    let config_path = dir.path().join("gary.json");
    std::fs::write(
        &config_path,
        serde_json::to_vec_pretty(&json!({
            "gateway": {
                "host": "127.0.0.1",
                "port": 9
            }
        }))
        .expect("config json"),
    )
    .expect("write config");

    let err = cmd_command_set(
        config_path.to_str().expect("config path"),
        "loop".to_owned(),
        Some("custom loop".to_owned()),
        None,
        true,
    )
    .await
    .expect_err("reserved command must fail");
    assert!(err.to_string().contains("collides"));
}

#[tokio::test]
async fn cmd_agent_create_posts_model_payload() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_agent_http_test_server(requests.clone(), StatusCode::OK).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_agent_create(
        config_path.to_str().expect("config path"),
        "spec-review".to_owned(),
        "Spec Review".to_owned(),
        "codex_app_server".to_owned(),
        Some("gpt-5".to_owned()),
        Some("/tmp/spec-review".to_owned()),
        "Review specs carefully.".to_owned(),
        false,
    )
    .await
    .expect("agent create should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].method, "POST");
    assert_eq!(records[0].path, "/api/custom-agents");
    assert_eq!(records[0].body["agent_id"], "spec-review");
    assert_eq!(records[0].body["model"], "gpt-5");
    assert_eq!(records[0].body["default_workspace_dir"], "/tmp/spec-review");
}

#[tokio::test]
async fn cmd_agent_update_puts_empty_model_when_omitted() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_agent_http_test_server(requests.clone(), StatusCode::OK).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_agent_update(
        config_path.to_str().expect("config path"),
        "spec-review".to_owned(),
        "Spec Review".to_owned(),
        "codex_app_server".to_owned(),
        None,
        None,
        "Review specs carefully.".to_owned(),
        false,
    )
    .await
    .expect("agent update should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].method, "PUT");
    assert_eq!(records[0].path, "/api/custom-agents/spec-review");
    assert_eq!(records[0].body["model"], "");
}

#[tokio::test]
async fn cmd_agent_upsert_falls_back_to_post_after_put_failure() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) =
        spawn_agent_http_test_server(requests.clone(), StatusCode::NOT_FOUND).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_agent_upsert(
        config_path.to_str().expect("config path"),
        "spec-review".to_owned(),
        "Spec Review".to_owned(),
        "gemini_cli".to_owned(),
        Some("gemini-3.1-pro-preview".to_owned()),
        None,
        "Review specs carefully.".to_owned(),
        false,
    )
    .await
    .expect("agent upsert should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].method, "PUT");
    assert_eq!(records[0].path, "/api/custom-agents/spec-review");
    assert_eq!(records[1].method, "POST");
    assert_eq!(records[1].path, "/api/custom-agents");
    assert_eq!(records[1].body["model"], "gemini-3.1-pro-preview");
}

#[test]
fn automation_schedule_args_build_interval_schedule() {
    let args = crate::cli::AutomationScheduleArgs {
        every_hours: Some(6),
        ..Default::default()
    };

    let schedule = automation_schedule_from_cli_args(&args, true)
        .expect("schedule parse")
        .expect("schedule");

    assert_eq!(schedule, AutomationScheduleView::Interval { hours: 6 });
}

#[test]
fn automation_schedule_args_build_daily_schedule() {
    let args = crate::cli::AutomationScheduleArgs {
        daily_time: Some("08:30".to_owned()),
        weekdays: vec!["mon".to_owned(), "fri".to_owned()],
        timezone: Some("Asia/Shanghai".to_owned()),
        ..Default::default()
    };

    let schedule = automation_schedule_from_cli_args(&args, true)
        .expect("schedule parse")
        .expect("schedule");

    assert_eq!(
        schedule,
        AutomationScheduleView::Daily {
            time: "08:30".to_owned(),
            weekdays: vec!["mon".to_owned(), "fri".to_owned()],
            timezone: "Asia/Shanghai".to_owned(),
        }
    );
}

#[test]
fn automation_schedule_args_reject_ambiguous_schedule_shape() {
    let args = crate::cli::AutomationScheduleArgs {
        every_hours: Some(6),
        once_at: Some("2030-05-01T08:30".to_owned()),
        ..Default::default()
    };

    let error = automation_schedule_from_cli_args(&args, true)
        .expect_err("ambiguous schedule should fail")
        .to_string();

    assert!(error.contains("choose exactly one schedule shape"));
}

#[tokio::test]
async fn cmd_automation_create_posts_disabled_interval_payload() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_automation_http_test_server(requests.clone()).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_automation_create(
        config_path.to_str().expect("config path"),
        "Daily triage".to_owned(),
        Some("Summarize repo state".to_owned()),
        Some("codex".to_owned()),
        Some(dir.path().to_string_lossy().to_string()),
        crate::cli::AutomationScheduleArgs {
            every_hours: Some(6),
            ..Default::default()
        },
        true,
        false,
    )
    .await
    .expect("automation create should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].method, "POST");
    assert_eq!(records[0].path, "/api/automations");
    assert_eq!(records[0].body["label"], "Daily triage");
    assert_eq!(records[0].body["prompt"], "Summarize repo state");
    assert_eq!(records[0].body["agentId"], "codex");
    assert_eq!(
        records[0].body["workspaceDir"].as_str(),
        Some(
            dir.path()
                .canonicalize()
                .expect("canonical tempdir")
                .to_string_lossy()
                .as_ref()
        )
    );
    assert_eq!(records[0].body["enabled"], false);
    assert_eq!(records[0].body["schedule"]["kind"], "interval");
    assert_eq!(records[0].body["schedule"]["hours"], 6);
}

#[tokio::test]
async fn cmd_automation_update_patches_requested_fields() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_automation_http_test_server(requests.clone()).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_automation_update(
        config_path.to_str().expect("config path"),
        "automation::created",
        Some("Weekly triage".to_owned()),
        None,
        None,
        None,
        crate::cli::AutomationScheduleArgs {
            daily_time: Some("09:45".to_owned()),
            timezone: Some("Asia/Shanghai".to_owned()),
            ..Default::default()
        },
        false,
        true,
        false,
    )
    .await
    .expect("automation update should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].method, "PATCH");
    assert_eq!(records[0].path, "/api/automations/automation::created");
    assert_eq!(records[0].body["label"], "Weekly triage");
    assert_eq!(records[0].body["enabled"], false);
    assert!(records[0].body.get("prompt").is_none());
    assert_eq!(records[0].body["schedule"]["kind"], "daily");
    assert_eq!(records[0].body["schedule"]["time"], "09:45");
    assert_eq!(records[0].body["schedule"]["timezone"], "Asia/Shanghai");
}

// Regression guard for the Weixin onboarding: `qrcode_img_content` from
// the iLink endpoint is a short URL, not ASCII art, so we must render a
// real QR locally. These tests pin that contract — a URL goes in, a
// multi-line block of Unicode half-block characters comes out.
#[test]
fn render_terminal_qr_produces_scannable_block_art() {
    let payload = "https://liteapp.weixin.qq.com/q/7GiQu1?qrcode=abc123&bot_type=3";
    let rendered = render_terminal_qr(payload).expect("QR should encode short URL");
    // For local debugging: `GARYX_TEST_SHOW_QR=1 cargo test -p garyx \
    //   render_terminal_qr_produces_scannable_block_art -- --nocapture`
    if std::env::var_os("GARYX_TEST_SHOW_QR").is_some() {
        eprintln!("\n--- Weixin QR sample ---\n{rendered}\n({payload})\n");
    }
    // Unicode half-blocks used by qrcode::render::unicode::Dense1x2.
    assert!(rendered.contains('\u{2580}') || rendered.contains('\u{2584}'));
    // Dense1x2 packs 2 module rows per line of text, so a ~29×29 QR
    // (version 3) becomes ~17 lines plus a 1-line quiet zone either
    // side. Accept anything ≥ 10 non-trivial rows — enough to prove
    // we produced a real block, not a single-line stub.
    let non_empty_rows = rendered
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    assert!(
        non_empty_rows >= 10,
        "expected at least ~10 rows, got {non_empty_rows}: \n{rendered}"
    );
}

#[test]
fn render_terminal_qr_returns_none_for_unencodable_input() {
    // QR version 40 maxes out at ~2953 bytes with ECL M. Feed it more.
    let huge = "x".repeat(8_000);
    assert!(render_terminal_qr(&huge).is_none());
}

#[test]
fn reauthorize_weixin_can_inherit_metadata_and_disable_previous_account() {
    let mut cfg = GaryxConfig::default();
    upsert_channel_account(
        &mut cfg,
        BUILTIN_CHANNEL_PLUGIN_WEIXIN,
        "old-wx",
        Some("Wiki".to_owned()),
        Some("/Users/test".to_owned()),
        Some("wiki-curator".to_owned()),
        Some("old-token".to_owned()),
        Some("old-uin".to_owned()),
        Some("https://ilinkai.weixin.qq.com".to_owned()),
        None,
        None,
        None,
        Map::new(),
    )
    .unwrap();

    let inherited = reauthorize_account_entry(&cfg, BUILTIN_CHANNEL_PLUGIN_WEIXIN, Some("old-wx"))
        .unwrap()
        .expect("previous account should exist");
    assert_eq!(inherited.name.as_deref(), Some("Wiki"));
    assert_eq!(inherited.workspace_dir.as_deref(), Some("/Users/test"));
    assert_eq!(inherited.agent_id.as_deref(), Some("wiki-curator"));
    assert_eq!(config_string(&inherited, "uin").as_deref(), Some("old-uin"));

    upsert_channel_account(
        &mut cfg,
        BUILTIN_CHANNEL_PLUGIN_WEIXIN,
        "new-wx",
        inherited.name.clone(),
        inherited.workspace_dir.clone(),
        inherited.agent_id.clone(),
        Some("new-token".to_owned()),
        config_string(&inherited, "uin"),
        Some("https://ilinkai.weixin.qq.com".to_owned()),
        None,
        None,
        None,
        Map::new(),
    )
    .unwrap();

    let action = finish_reauthorization(
        &mut cfg,
        BUILTIN_CHANNEL_PLUGIN_WEIXIN,
        Some("old-wx"),
        "new-wx",
        false,
    )
    .unwrap();
    assert_eq!(action, Some("disabled"));

    let accounts = &cfg
        .channels
        .plugin_channel(BUILTIN_CHANNEL_PLUGIN_WEIXIN)
        .unwrap()
        .accounts;
    assert!(!accounts["old-wx"].enabled);
    assert!(accounts["new-wx"].enabled);
    assert_eq!(accounts["new-wx"].name.as_deref(), Some("Wiki"));
    assert_eq!(
        accounts["new-wx"].workspace_dir.as_deref(),
        Some("/Users/test")
    );
    assert_eq!(accounts["new-wx"].agent_id.as_deref(), Some("wiki-curator"));
    assert_eq!(accounts["new-wx"].config["uin"], "old-uin");
    assert_eq!(accounts["new-wx"].config["token"], "new-token");
}

#[test]
fn next_onboard_steps_suggests_channel_bind_when_empty() {
    let cfg = GaryxConfig::default();
    let steps = next_onboard_steps(&cfg);
    assert!(
        steps.iter().any(|s| s.contains("garyx channels add")),
        "expected channel-add hint in fresh config, got {steps:?}"
    );
}

#[test]
fn next_onboard_steps_omits_channel_bind_when_user_channel_exists() {
    let mut cfg = GaryxConfig::default();
    cfg.channels.plugin_channel_mut("telegram").accounts.insert(
        "alice".to_owned(),
        telegram_account_to_plugin_entry(&TelegramAccount {
            token: "t".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            groups: Default::default(),
        }),
    );
    let steps = next_onboard_steps(&cfg);
    assert!(
        !steps.iter().any(|s| s.contains("garyx channels add")),
        "should not nag about binding when a channel already exists, got {steps:?}"
    );
}

#[test]
fn user_channel_account_count_ignores_api_accounts() {
    let mut cfg = GaryxConfig::default();
    cfg.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
        },
    );
    // api-only should still count as zero user-facing channels
    assert_eq!(user_channel_account_count(&cfg), 0);
}

#[test]
fn normalize_release_version_strips_leading_v() {
    assert_eq!(normalize_release_version("v0.1.6"), "0.1.6");
    assert_eq!(normalize_release_version("0.1.6"), "0.1.6");
    assert_eq!(normalize_release_version("  v1.2.3-rc.1  "), "1.2.3-rc.1");
}

#[test]
fn detect_release_target_for_supported_platforms() {
    assert_eq!(
        detect_release_target_for("macos", "aarch64").expect("mac arm64 target"),
        "aarch64-apple-darwin"
    );
    assert_eq!(
        detect_release_target_for("linux", "x86_64").expect("linux x64 target"),
        "x86_64-unknown-linux-gnu"
    );
    assert!(detect_release_target_for("windows", "x86_64").is_err());
}

#[test]
fn parse_sha256_checksum_accepts_standard_release_file() {
    let checksum = parse_sha256_checksum(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  garyx-0.1.6-aarch64-apple-darwin.tar.gz\n",
        )
        .expect("checksum");
    assert_eq!(
        checksum,
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    );
}

#[tokio::test]
async fn channels_add_persists_generic_plugin_accounts() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let plugin_root = tempdir().expect("plugin root");
    let _env = ScopedEnvVar::set_path("GARYX_PLUGIN_DIR", plugin_root.path());
    write_test_plugin_bundle(plugin_root.path(), "test-acmechat-cli", &["token"]);

    let config_dir = tempdir().expect("config dir");
    let config_path = write_empty_config_file(&config_dir);

    cmd_channels_add(
        config_path.to_str().expect("config path"),
        Some("test-acmechat-cli".to_owned()),
        Some("agent-1".to_owned()),
        Some("AcmeChat Main".to_owned()),
        None,
        None,
        Some("tok-1".to_owned()),
        None,
        Some("https://chat.example.com".to_owned()),
        None,
        None,
        None,
        false,
    )
    .await
    .expect("plugin add should succeed");

    let loaded = load_config_or_default(
        config_path.to_str().expect("config path"),
        ConfigRuntimeOverrides::default(),
    )
    .expect("load config");
    let entry = loaded
        .config
        .channels
        .plugins
        .get("test-acmechat-cli")
        .and_then(|plugin| plugin.accounts.get("agent-1"))
        .expect("plugin account should exist");
    assert_eq!(entry.name.as_deref(), Some("AcmeChat Main"));
    assert_eq!(entry.agent_id.as_deref(), Some("claude"));
    assert_eq!(entry.config["token"], "tok-1");
    assert_eq!(entry.config["base_url"], "https://chat.example.com");
}

#[test]
fn upsert_plugin_account_rejects_missing_required_fields() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let plugin_root = tempdir().expect("plugin root");
    let _env = ScopedEnvVar::set_path("GARYX_PLUGIN_DIR", plugin_root.path());
    write_test_plugin_bundle(plugin_root.path(), "test-acmechat-cli", &["token"]);

    let mut cfg = GaryxConfig::default();
    let err = upsert_channel_account(
        &mut cfg,
        "test-acmechat-cli",
        "agent-1",
        None,
        None,
        None,
        None,
        None,
        Some("https://chat.example.com".to_owned()),
        None,
        None,
        None,
        Map::new(),
    )
    .expect_err("missing token should fail");
    assert!(
        err.to_string().contains("missing required fields"),
        "unexpected error: {err}"
    );
}

#[test]
fn task_create_assignee_accepts_bare_agent_id() {
    let payload = task_create_assignee_payload(Some(" plain-claude ")).unwrap();

    assert_eq!(
        payload,
        Some(json!({ "kind": "agent", "agent_id": "plain-claude" }))
    );
}

#[test]
fn task_runtime_agent_id_is_derived_from_agent_assignee() {
    let payload = task_create_assignee_payload(Some("agent:reviewer")).unwrap();

    assert_eq!(
        task_runtime_agent_id_from_assignee(&payload).as_deref(),
        Some("reviewer")
    );
}

#[test]
fn task_runtime_agent_id_is_not_derived_from_human_assignee() {
    let payload = task_create_assignee_payload(Some("human:alice")).unwrap();

    assert_eq!(task_runtime_agent_id_from_assignee(&payload), None);
}

#[test]
fn format_task_progress_groups_each_user_turn_with_last_assistant_text_group() {
    let task_payload = json!({
        "task_ref": "#TASK-42",
        "thread_id": "thread::task-42",
        "task": {
            "title": "Ship task progress",
            "status": "done",
            "assignee": {"kind": "agent", "agent_id": "claude"},
            "updated_by": {"kind": "agent", "agent_id": "claude"}
        },
        "thread": {
            "messages": [
                {"role": "user", "content": "original request", "timestamp": "2026-05-03T00:00:00Z"}
            ]
        }
    });
    let history_payload = json!({
        "messages": [
            {
                "role": "user",
                "text": "please do it",
                "timestamp": "2026-05-03T00:00:01Z",
                "internal": false
            },
            {
                "role": "assistant",
                "text": "first text before tools",
                "timestamp": "2026-05-03T00:00:02Z"
            },
            {
                "role": "tool_use",
                "text": "Bash",
                "timestamp": "2026-05-03T00:00:03Z",
                "tool_related": true
            },
            {
                "role": "assistant",
                "text": "final answer after tools",
                "timestamp": "2026-05-03T00:00:04Z"
            },
            {
                "role": "user",
                "text": "follow up",
                "timestamp": "2026-05-03T00:00:05Z",
                "internal": true
            }
        ]
    });

    let rendered = format_task_progress(&task_payload, Some(&history_payload));

    assert!(rendered.contains("Task: #TASK-42"));
    assert!(rendered.contains("[1] User 2026-05-03T00:00:00Z"));
    assert!(rendered.contains("original request"));
    assert!(rendered.contains("[2] User 2026-05-03T00:00:01Z"));
    assert!(rendered.contains("please do it"));
    assert!(rendered.contains("final answer after tools"));
    assert!(
        !rendered.contains("first text before tools"),
        "only the last assistant text group after a user turn should render: {rendered}"
    );
    assert!(rendered.contains("[3] User 2026-05-03T00:00:05Z"));
    assert!(rendered.contains("(internal dispatch)"));
    assert!(rendered.contains(
        "Full thread with tool calls: garyx thread history thread::task-42 --limit 200 --json"
    ));
}

#[test]
fn task_progress_turns_keeps_last_consecutive_assistant_group() {
    let messages = vec![
        TaskProgressMessage {
            role: "user".to_owned(),
            text: "u1".to_owned(),
            timestamp: None,
            sort_time: None,
            source_order: 0,
            internal: false,
        },
        TaskProgressMessage {
            role: "assistant".to_owned(),
            text: "a1".to_owned(),
            timestamp: None,
            sort_time: None,
            source_order: 1,
            internal: false,
        },
        TaskProgressMessage {
            role: "assistant".to_owned(),
            text: "a2".to_owned(),
            timestamp: None,
            sort_time: None,
            source_order: 2,
            internal: false,
        },
    ];

    let turns = task_progress_turns(&messages);

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].assistant_text.as_deref(), Some("a1\n\na2"));
}
