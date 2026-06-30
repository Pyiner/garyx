#![allow(clippy::await_holding_lock)]

use super::*;
use axum::{
    Json, Router,
    extract::Path as AxumPath,
    http::StatusCode,
    routing::{get, patch, post, put},
};
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

#[test]
fn resolve_cli_message_bot_prefers_explicit_bot() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _bot = ScopedEnvVar::set_string("GARYX_BOT", "telegram:env");

    let bot = resolve_cli_message_bot(Some("telegram:explicit")).unwrap();

    assert_eq!(bot, "telegram:explicit");
}

#[test]
fn resolve_cli_message_bot_uses_env_bot() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _bot = ScopedEnvVar::set_string("GARYX_BOT", "telegram:main");
    let _channel = ScopedEnvVar::remove("GARYX_CHANNEL");
    let _account = ScopedEnvVar::remove("GARYX_ACCOUNT_ID");

    let bot = resolve_cli_message_bot(None).unwrap();

    assert_eq!(bot, "telegram:main");
}

#[test]
fn resolve_cli_message_bot_uses_channel_account_env() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _bot = ScopedEnvVar::remove("GARYX_BOT");
    let _channel = ScopedEnvVar::set_string("GARYX_CHANNEL", "telegram");
    let _account = ScopedEnvVar::set_string("GARYX_ACCOUNT_ID", "main");

    let bot = resolve_cli_message_bot(None).unwrap();

    assert_eq!(bot, "telegram:main");
}

#[test]
fn resolve_cli_message_bot_requires_bot_context() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _bot = ScopedEnvVar::remove("GARYX_BOT");
    let _channel = ScopedEnvVar::remove("GARYX_CHANNEL");
    let _account = ScopedEnvVar::remove("GARYX_ACCOUNT_ID");

    let error = resolve_cli_message_bot(None).unwrap_err().to_string();

    assert!(error.contains("bot is required"));
}

#[test]
fn normalize_bot_selector_arg_accepts_channel_account_selector() {
    let bot = normalize_bot_selector_arg(" telegram:main ").unwrap();

    assert_eq!(bot, "telegram:main");
}

#[test]
fn normalize_bot_selector_arg_rejects_invalid_selector() {
    let error = normalize_bot_selector_arg("telegram")
        .unwrap_err()
        .to_string();

    assert!(error.contains("channel:account_id"));
}

#[test]
fn normalize_thread_id_arg_requires_canonical_thread_id() {
    let error = normalize_thread_id_arg("not-a-thread")
        .unwrap_err()
        .to_string();

    assert!(error.contains("thread::"));
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

async fn spawn_settings_update_http_test_server(
    requests: StdArc<Mutex<Vec<RecordedRequest>>>,
) -> (String, JoinHandle<()>) {
    let put_requests = requests.clone();
    let app = Router::new().route(
        "/api/settings",
        put(move |uri: axum::http::Uri, Json(payload): Json<Value>| {
            let requests = put_requests.clone();
            async move {
                requests
                    .lock()
                    .expect("request lock")
                    .push(RecordedRequest {
                        method: "PUT".to_owned(),
                        path: uri
                            .path_and_query()
                            .map(|value| value.as_str().to_owned())
                            .unwrap_or_else(|| "/api/settings".to_owned()),
                        body: payload,
                    });
                (StatusCode::OK, Json(json!({"ok": true})))
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
    let trigger_list_requests = requests.clone();
    let trigger_create_requests = requests.clone();
    let trigger_patch_requests = requests.clone();
    let trigger_delete_requests = requests.clone();

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
        )
        .route(
            "/api/automations/triggers/data",
            get(move || {
                let requests = trigger_list_requests.clone();
                async move {
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "GET".to_owned(),
                            path: "/api/automations/triggers/data".to_owned(),
                            body: Value::Null,
                        });
                    (
                        StatusCode::OK,
                        Json(json!({
                            "triggers": []
                        })),
                    )
                }
            })
            .post(move |Json(payload): Json<Value>| {
                let requests = trigger_create_requests.clone();
                async move {
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "POST".to_owned(),
                            path: "/api/automations/triggers/data".to_owned(),
                            body: payload.clone(),
                        });
                    (
                        StatusCode::CREATED,
                        Json(json!({
                                "trigger": {
                                    "id": "autodata_test",
                                    "label": payload["label"],
                                    "tableName": payload["tableName"],
                                    "eventType": payload["eventType"],
                                "titleTemplate": payload["titleTemplate"],
                                "bodyTemplate": payload["bodyTemplate"],
                                "agentId": payload.get("agentId").cloned().unwrap_or(Value::Null),
                                "workspaceDir": payload.get("workspaceDir").cloned().unwrap_or(Value::Null),
                                "enabled": payload.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                                "createdAt": "2030-05-01T08:30:00Z",
                                "updatedAt": "2030-05-01T08:30:00Z"
                            }
                        })),
                    )
                }
            }),
        )
        .route(
            "/api/automations/triggers/data/{trigger_id}",
            patch(
                move |AxumPath(trigger_id): AxumPath<String>, Json(payload): Json<Value>| {
                    let requests = trigger_patch_requests.clone();
                    async move {
                        let path = format!("/api/automations/triggers/data/{trigger_id}");
                        requests.lock().expect("request lock").push(RecordedRequest {
                            method: "PATCH".to_owned(),
                            path,
                            body: payload.clone(),
                        });
                        (
                            StatusCode::OK,
                            Json(json!({
                                "trigger": {
                                    "id": trigger_id,
                                    "tableName": "contacts",
                                    "eventType": "record.created",
                                    "label": "Contact review",
                                    "titleTemplate": "New record {record_id}",
                                    "bodyTemplate": "Review {table_name}",
                                    "enabled": payload["enabled"],
                                    "createdAt": "2030-05-01T08:30:00Z",
                                    "updatedAt": "2030-05-01T08:31:00Z"
                                }
                            })),
                        )
                    }
                },
            )
            .delete(move |AxumPath(trigger_id): AxumPath<String>| {
                let requests = trigger_delete_requests.clone();
                async move {
                    let path = format!("/api/automations/triggers/data/{trigger_id}");
                    requests.lock().expect("request lock").push(RecordedRequest {
                        method: "DELETE".to_owned(),
                        path,
                        body: Value::Null,
                    });
                    (
                        StatusCode::OK,
                        Json(json!({
                            "deleted": true,
                            "id": trigger_id,
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

async fn spawn_thread_task_http_test_server(
    requests: StdArc<Mutex<Vec<RecordedRequest>>>,
) -> (String, JoinHandle<()>) {
    let thread_requests = requests.clone();
    let task_requests = requests.clone();
    let app = Router::new()
        .route(
            "/api/threads",
            post(move |Json(payload): Json<Value>| {
                let requests = thread_requests.clone();
                async move {
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "POST".to_owned(),
                            path: "/api/threads".to_owned(),
                            body: payload.clone(),
                        });
                    (
                        StatusCode::CREATED,
                        Json(json!({
                            "thread_id": "thread::test",
                            "thread_key": "thread::test",
                            "label": payload["label"],
                            "workspace_dir": payload["workspaceDir"],
                            "message_count": 0,
                        })),
                    )
                }
            }),
        )
        .route(
            "/api/tasks",
            post(move |Json(payload): Json<Value>| {
                let requests = task_requests.clone();
                async move {
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "POST".to_owned(),
                            path: "/api/tasks".to_owned(),
                            body: payload.clone(),
                        });
                    (
                        StatusCode::CREATED,
                        Json(json!({
                            "thread_id": "thread::task",
                            "task_id": "#TASK-1",
                            "number": 1,
                            "status": "todo",
                            "runtime_agent_id": "claude",
                            "task": {
                                "schema_version": "garyx.task.v1",
                                "number": 1,
                                "title": payload["title"],
                                "status": "todo",
                                "creator": {"kind": "human", "id": "cli"},
                                "updated_by": {"kind": "human", "id": "cli"},
                                "created_at": "2030-01-01T00:00:00Z",
                                "updated_at": "2030-01-01T00:00:00Z",
                                "events": []
                            }
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
async fn cmd_config_provider_model_puts_settings_patch() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_settings_update_http_test_server(requests.clone()).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_config_provider_model(
        config_path.to_str().expect("config path"),
        "claude_code",
        Some("claude-opus-4-8".to_owned()),
        false,
        Some("max".to_owned()),
        false,
        None,
        false,
        None,
        false,
        true,
    )
    .await
    .expect("provider model update should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].method, "PUT");
    assert_eq!(records[0].path, "/api/settings?merge=true");
    assert_eq!(
        records[0].body["agents"]["claude"]["provider_type"],
        "claude_code"
    );
    assert_eq!(
        records[0].body["agents"]["claude"]["default_model"],
        "claude-opus-4-8"
    );
    assert_eq!(
        records[0].body["agents"]["claude"]["model_reasoning_effort"],
        "max"
    );
}

#[tokio::test]
async fn cmd_config_provider_model_clears_native_provider_defaults() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_settings_update_http_test_server(requests.clone()).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_config_provider_model(
        config_path.to_str().expect("config path"),
        "anthropic",
        None,
        true,
        None,
        true,
        None,
        false,
        None,
        false,
        true,
    )
    .await
    .expect("provider model clear should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].method, "PUT");
    assert_eq!(records[0].path, "/api/settings?merge=true");
    assert_eq!(
        records[0].body["agents"]["anthropic"]["provider_type"],
        "anthropic"
    );
    assert_eq!(records[0].body["agents"]["anthropic"]["default_model"], "");
    assert_eq!(
        records[0].body["agents"]["anthropic"]["model_reasoning_effort"],
        ""
    );
}

#[tokio::test]
async fn cmd_config_provider_model_rejects_unknown_provider_without_request() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_settings_update_http_test_server(requests.clone()).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    let error = cmd_config_provider_model(
        config_path.to_str().expect("config path"),
        "unknown_provider",
        Some("model-x".to_owned()),
        false,
        None,
        false,
        None,
        false,
        None,
        false,
        true,
    )
    .await
    .expect_err("unknown provider should fail");

    handle.abort();

    assert!(
        error
            .to_string()
            .contains("unsupported provider type: unknown_provider")
    );
    assert!(requests.lock().expect("request lock").is_empty());
}

#[tokio::test]
async fn cmd_config_provider_model_puts_claude_cli_mode_patch() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_settings_update_http_test_server(requests.clone()).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_config_provider_model(
        config_path.to_str().expect("config path"),
        "claude_code",
        None,
        false,
        None,
        false,
        Some("cctty".to_owned()),
        false,
        Some("/opt/garyx/bin/custom-cctty".to_owned()),
        false,
        true,
    )
    .await
    .expect("claude cli mode update should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].method, "PUT");
    assert_eq!(records[0].path, "/api/settings?merge=true");
    assert_eq!(
        records[0].body["agents"]["claude"]["provider_type"],
        "claude_code"
    );
    assert_eq!(
        records[0].body["agents"]["claude"]["claude_cli_mode"],
        "cctty"
    );
    assert_eq!(
        records[0].body["agents"]["claude"]["claude_cli_path"],
        "/opt/garyx/bin/custom-cctty"
    );
}

#[tokio::test]
async fn cmd_config_provider_model_rejects_claude_cli_mode_for_other_providers() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_settings_update_http_test_server(requests.clone()).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    let error = cmd_config_provider_model(
        config_path.to_str().expect("config path"),
        "codex_app_server",
        None,
        false,
        None,
        false,
        Some("cctty".to_owned()),
        false,
        None,
        false,
        true,
    )
    .await
    .expect_err("claude cli options should be claude-only");

    handle.abort();

    assert!(
        error
            .to_string()
            .contains("only supported for provider claude_code")
    );
    assert!(requests.lock().expect("request lock").is_empty());
}

#[test]
fn provider_model_config_key_maps_configurable_provider_types() {
    assert_eq!(
        provider_model_config_key(&ProviderType::ClaudeCode).unwrap(),
        "claude"
    );
    assert_eq!(
        provider_model_config_key(&ProviderType::CodexAppServer).unwrap(),
        "codex"
    );
    assert_eq!(
        provider_model_config_key(&ProviderType::GeminiCli).unwrap(),
        "gemini"
    );
    assert_eq!(
        provider_model_config_key(&ProviderType::AntigravityCli).unwrap(),
        "antigravity"
    );
    assert_eq!(
        provider_model_config_key(&ProviderType::Gpt).unwrap(),
        "gpt"
    );
    assert_eq!(
        provider_model_config_key(&ProviderType::ClaudeLlm).unwrap(),
        "anthropic"
    );
    assert_eq!(
        provider_model_config_key(&ProviderType::GeminiLlm).unwrap(),
        "google"
    );
    assert!(provider_model_config_key(&ProviderType::AgentTeam).is_err());
}

#[tokio::test]
async fn cmd_thread_create_posts_worktree_mode() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_thread_task_http_test_server(requests.clone()).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_thread_create(
        config_path.to_str().expect("config path"),
        Some("Worktree thread".to_owned()),
        Some("/tmp/garyx-repo".to_owned()),
        Some("claude".to_owned()),
        true,
        true,
    )
    .await
    .expect("thread create should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].method, "POST");
    assert_eq!(records[0].path, "/api/threads");
    assert_eq!(records[0].body["label"], "Worktree thread");
    assert_eq!(records[0].body["workspaceDir"], "/tmp/garyx-repo");
    assert_eq!(records[0].body["agentId"], "claude");
    assert_eq!(records[0].body["workspaceMode"], "worktree");
}

#[tokio::test]
async fn cmd_task_create_posts_worktree_runtime_mode() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_thread_task_http_test_server(requests.clone()).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_task_create(
        config_path.to_str().expect("config path"),
        Some("Task worktree".to_owned()),
        Some("Do the work".to_owned()),
        Some("agent:claude"),
        false,
        Some("/tmp/garyx-repo".to_owned()),
        true,
        None,
        None,
        None,
        None,
        None,
        None,
        vec!["none".to_owned()],
        true,
    )
    .await
    .expect("task create should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].method, "POST");
    assert_eq!(records[0].path, "/api/tasks");
    assert_eq!(records[0].body["title"], "Task worktree");
    assert_eq!(
        records[0].body["runtime"]["workspace_dir"],
        "/tmp/garyx-repo"
    );
    assert_eq!(records[0].body["runtime"]["workspace_mode"], "worktree");
}

#[tokio::test]
async fn cmd_task_create_posts_agent_executor() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_thread_task_http_test_server(requests.clone()).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_task_create(
        config_path.to_str().expect("config path"),
        Some("Agent task".to_owned()),
        Some("Do the work".to_owned()),
        None,
        false,
        Some("/tmp/garyx-repo".to_owned()),
        false,
        Some("claude".to_owned()),
        None,
        None,
        None,
        None,
        None,
        vec!["none".to_owned()],
        true,
    )
    .await
    .expect("task create should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].body["executor"]["type"], "agent");
    assert_eq!(records[0].body["executor"]["agentId"], "claude");
    assert_eq!(records[0].body["assignee"], Value::Null);
    assert_eq!(records[0].body["start"], true);
    assert_eq!(
        records[0].body["runtime"]["workspace_dir"],
        "/tmp/garyx-repo"
    );
}

#[tokio::test]
async fn cmd_task_create_posts_team_executor() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_thread_task_http_test_server(requests.clone()).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_task_create(
        config_path.to_str().expect("config path"),
        Some("Team task".to_owned()),
        None,
        None,
        false,
        None,
        false,
        None,
        Some("product-ship".to_owned()),
        None,
        None,
        None,
        None,
        vec!["none".to_owned()],
        true,
    )
    .await
    .expect("task create should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].body["executor"]["type"], "team");
    assert_eq!(records[0].body["executor"]["teamId"], "product-ship");
    assert_eq!(records[0].body["assignee"], Value::Null);
    assert_eq!(records[0].body["start"], true);
}

#[tokio::test]
async fn cmd_task_create_rejects_executor_with_assignee() {
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, "http://127.0.0.1:9");

    let error = cmd_task_create(
        config_path.to_str().expect("config path"),
        Some("Mixed task".to_owned()),
        None,
        Some("agent:reviewer"),
        false,
        None,
        false,
        Some("claude".to_owned()),
        None,
        None,
        None,
        None,
        None,
        vec!["none".to_owned()],
        true,
    )
    .await
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("executor cannot be combined with --assignee")
    );
}

#[tokio::test]
async fn cmd_task_create_posts_workflow_workspace_at_top_level() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_thread_task_http_test_server(requests.clone()).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_task_create(
        config_path.to_str().expect("config path"),
        Some("Workflow task".to_owned()),
        None,
        None,
        false,
        Some("/tmp/garyx-workflow".to_owned()),
        false,
        None,
        None,
        Some("smoke".to_owned()),
        None,
        None,
        Some(r#"{"question":"smoke"}"#.to_owned()),
        vec!["none".to_owned()],
        true,
    )
    .await
    .expect("task create should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].method, "POST");
    assert_eq!(records[0].path, "/api/tasks");
    assert_eq!(records[0].body["executor"]["type"], "workflow");
    assert_eq!(records[0].body["executor"]["workflowId"], "smoke");
    assert_eq!(records[0].body["executor"]["input"]["question"], "smoke");
    assert_eq!(records[0].body["workspace_dir"], "/tmp/garyx-workflow");
    assert_eq!(
        records[0].body["runtime"]["workspace_dir"],
        "/tmp/garyx-workflow"
    );
}

#[tokio::test]
async fn cmd_task_create_posts_workflow_plain_text_input() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_thread_task_http_test_server(requests.clone()).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_task_create(
        config_path.to_str().expect("config path"),
        Some("Workflow text task".to_owned()),
        None,
        None,
        false,
        None,
        false,
        None,
        None,
        Some("smoke".to_owned()),
        Some("Summarize this bug report".to_owned()),
        None,
        None,
        vec!["none".to_owned()],
        true,
    )
    .await
    .expect("task create should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].body["executor"]["type"], "workflow");
    assert_eq!(records[0].body["executor"]["workflowId"], "smoke");
    assert_eq!(
        records[0].body["executor"]["input"],
        "Summarize this bug report"
    );
}

#[tokio::test]
async fn cmd_task_create_posts_workflow_input_file_as_plain_text() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_thread_task_http_test_server(requests.clone()).await;
    let dir = tempdir().expect("tempdir");
    let input_path = dir.path().join("workflow-input.txt");
    std::fs::write(&input_path, "First line\nSecond line\n").expect("input file");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_task_create(
        config_path.to_str().expect("config path"),
        Some("Workflow file task".to_owned()),
        None,
        None,
        false,
        None,
        false,
        None,
        None,
        Some("smoke".to_owned()),
        None,
        Some(input_path),
        None,
        vec!["none".to_owned()],
        true,
    )
    .await
    .expect("task create should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].body["executor"]["type"], "workflow");
    assert_eq!(
        records[0].body["executor"]["input"],
        "First line\nSecond line\n"
    );
}

#[test]
fn workflow_package_install_copies_manifest_and_code_into_config_root() {
    let temp = tempdir().expect("tempdir");
    let package = temp.path().join("source").join("smoke");
    std::fs::create_dir_all(&package).expect("source dirs");
    std::fs::write(package.join("workflow.ts"), "export {};\n").expect("entrypoint");
    std::fs::write(
        package.join(WORKFLOW_MANIFEST_FILE),
        r#"{
          "workflowId": "smoke",
          "version": 1,
          "name": "Smoke",
          "input": {"placeholder": "Smoke request"}
        }"#,
    )
    .expect("manifest");
    let (source, manifest) = workflow_package_source(&package).expect("source");
    assert_eq!(source, package);
    assert_eq!(manifest, package.join(WORKFLOW_MANIFEST_FILE));

    let mut config = GaryxConfig::default();
    config.sessions.data_dir = Some(temp.path().join("data").to_string_lossy().to_string());
    let destination = workflow_definitions_root_for_config(&config).join("smoke");
    install_workflow_package(&source, &destination).expect("install");
    assert!(destination.join(WORKFLOW_MANIFEST_FILE).is_file());
    assert!(destination.join("workflow.ts").is_file());
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
        "threads".to_owned(),
        Some("custom thread list".to_owned()),
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
        Some("high".to_owned()),
        Some("priority".to_owned()),
        None,
        None,
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
    assert_eq!(records[0].body["model_reasoning_effort"], "high");
    assert_eq!(records[0].body["model_service_tier"], "priority");
    assert_eq!(records[0].body["default_workspace_dir"], "/tmp/spec-review");
}

#[tokio::test]
async fn cmd_agent_update_omits_model_fields_when_omitted() {
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
        false,
        None,
        None,
        None,
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
    assert!(records[0].body.get("model").is_none());
    assert!(records[0].body.get("model_reasoning_effort").is_none());
    assert!(records[0].body.get("model_service_tier").is_none());
}

#[tokio::test]
async fn cmd_agent_update_sends_empty_model_when_clear_model_is_set() {
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
        true,
        None,
        None,
        None,
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
    assert!(records[0].body.get("model_reasoning_effort").is_none());
    assert!(records[0].body.get("model_service_tier").is_none());
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
        false,
        None,
        None,
        None,
        None,
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

#[tokio::test]
async fn cmd_agent_create_posts_native_provider_api_key_payload() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_agent_http_test_server(requests.clone(), StatusCode::OK).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_agent_create(
        config_path.to_str().expect("config path"),
        "budget-gpt".to_owned(),
        "Budget GPT".to_owned(),
        "gpt".to_owned(),
        Some("gpt-5.5".to_owned()),
        Some("medium".to_owned()),
        None,
        None,
        Some("test-openai-api-key".to_owned()),
        None,
        "Use GPT.".to_owned(),
        false,
    )
    .await
    .expect("agent create should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].body["provider_type"], "gpt");
    assert_eq!(records[0].body["auth_source"], "api_key");
    assert_eq!(
        records[0].body["provider_env"]["OPENAI_API_KEY"],
        "test-openai-api-key"
    );
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
        None,
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
async fn cmd_automation_data_trigger_create_posts_automation_payload() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_automation_http_test_server(requests.clone()).await;
    let dir = tempdir().expect("tempdir");
    let config_path = write_test_gateway_config(&dir, &base_url);

    cmd_automation_data_trigger_create(
        config_path.to_str().expect("config path"),
        "contacts",
        "record.created",
        "Contact review",
        "New record {record_id}",
        "Review {table_name}",
        Some("codex".to_owned()),
        Some("/tmp/work".to_owned()),
        true,
        false,
    )
    .await
    .expect("automation data trigger create should succeed");

    handle.abort();

    let records = requests.lock().expect("request lock");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].method, "POST");
    assert_eq!(records[0].path, "/api/automations/triggers/data");
    assert_eq!(records[0].body["tableName"], "contacts");
    assert_eq!(records[0].body["eventType"], "record.created");
    assert_eq!(records[0].body["label"], "Contact review");
    assert_eq!(records[0].body["titleTemplate"], "New record {record_id}");
    assert_eq!(records[0].body["bodyTemplate"], "Review {table_name}");
    assert_eq!(records[0].body["agentId"], "codex");
    assert_eq!(records[0].body["workspaceDir"], "/tmp/work");
    assert_eq!(records[0].body["enabled"], false);
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
        Some("worktree".to_owned()),
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
    assert_eq!(inherited.workspace_mode.as_deref(), Some("worktree"));
    assert_eq!(inherited.agent_id.as_deref(), Some("wiki-curator"));
    assert_eq!(config_string(&inherited, "uin").as_deref(), Some("old-uin"));

    upsert_channel_account(
        &mut cfg,
        BUILTIN_CHANNEL_PLUGIN_WEIXIN,
        "new-wx",
        inherited.name.clone(),
        inherited.workspace_dir.clone(),
        inherited.workspace_mode.clone(),
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
    assert!(
        steps.iter().any(|s| s == "garyx status"),
        "expected status check after onboarding, got {steps:?}"
    );
    assert!(
        !steps.iter().any(|s| s.contains("gateway install")),
        "onboarding should assume the gateway service is already installed, got {steps:?}"
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
            workspace_mode: None,
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
fn macos_cli_codesign_args_use_stable_identifier() {
    let args = macos_cli_codesign_args(Path::new("/tmp/garyx"));
    let args = args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    assert_eq!(
        args,
        vec![
            "--force",
            "--sign",
            "-",
            "--identifier",
            "com.garyx.gateway",
            "/tmp/garyx"
        ]
    );
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

// ---------------------------------------------------------------------------
// B1: staged-binary version verification (pre-rename gate).
// ---------------------------------------------------------------------------

#[test]
fn parse_self_reported_version_extracts_from_clap_output() {
    // `garyx <version>` is the clap default printed by the root command
    // and the B0 short-circuit. We take the trailing token of the first
    // non-empty line and strip any leading `v`.
    assert_eq!(
        parse_self_reported_version("garyx 0.1.32\n").as_deref(),
        Some("0.1.32")
    );
    assert_eq!(
        parse_self_reported_version("garyx v0.1.32").as_deref(),
        Some("0.1.32")
    );
    assert_eq!(
        parse_self_reported_version("garyx 0.1.33-rc.1\n").as_deref(),
        Some("0.1.33-rc.1")
    );
    // Leading blank line(s) skipped to the first real line.
    assert_eq!(
        parse_self_reported_version("\n\ngaryx 0.1.32\n").as_deref(),
        Some("0.1.32")
    );
}

#[test]
fn parse_self_reported_version_rejects_empty_or_blank() {
    assert_eq!(parse_self_reported_version(""), None);
    assert_eq!(parse_self_reported_version("   \n\t\n"), None);
    // A lone `v` normalizes to empty and is rejected.
    assert_eq!(parse_self_reported_version("garyx v"), None);
}

#[test]
fn verify_staged_version_accepts_exact_match() {
    assert_eq!(
        verify_staged_version(Some("0.1.32"), "0.1.32"),
        Ok("0.1.32".to_owned())
    );
}

#[test]
fn verify_staged_version_rejects_mismatch_with_typed_error() {
    // The canonical "version loop" shape: requested tag advanced but
    // the staged binary still self-reports the old version.
    assert_eq!(
        verify_staged_version(Some("0.1.29"), "0.1.32"),
        Err(SwapError::VersionMismatch {
            measured: "0.1.29".to_owned(),
            expected: "0.1.32".to_owned(),
        })
    );
}

#[test]
fn verify_staged_version_uses_exact_equality_not_greater_or_equal() {
    // A binary that self-reports a HIGHER version than the requested
    // tag must still be rejected: the contract is "self-reports the tag
    // it was published under", not ">=". This is the exact-match
    // prerelease guard (spec test 6).
    assert_eq!(
        verify_staged_version(Some("0.1.33"), "0.1.32"),
        Err(SwapError::VersionMismatch {
            measured: "0.1.33".to_owned(),
            expected: "0.1.32".to_owned(),
        })
    );
    // And prerelease exact-match passes.
    assert_eq!(
        verify_staged_version(Some("0.1.33-rc.1"), "0.1.33-rc.1"),
        Ok("0.1.33-rc.1".to_owned())
    );
    assert_eq!(
        verify_staged_version(Some("0.1.33"), "0.1.33-rc.1"),
        Err(SwapError::VersionMismatch {
            measured: "0.1.33".to_owned(),
            expected: "0.1.33-rc.1".to_owned(),
        })
    );
}

#[test]
fn verify_staged_version_missing_measurement_is_probe_failure() {
    assert_eq!(
        verify_staged_version(None, "0.1.32"),
        Err(SwapError::ProbeFailed {
            reason: "empty or malformed --version output".to_owned(),
        })
    );
}

/// Write an executable fake "staged binary" shell script that prints
/// `body` to stdout, exits with `exit_code`, and — to prove HOME
/// isolation — creates `$HOME/.garyx/probe-marker` as a side effect
/// before exiting. The marker lets a test assert the probe ran the
/// binary under an isolated HOME rather than the caller's real home.
#[cfg(unix)]
fn write_fake_staged_binary(dir: &Path, body: &str, exit_code: i32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(format!(".garyx-update-{}.tmp", Uuid::new_v4().simple()));
    let script = format!(
        "#!/bin/sh\nmkdir -p \"$HOME/.garyx\"\n: > \"$HOME/.garyx/probe-marker\"\n{body}\nexit {exit_code}\n"
    );
    std::fs::write(&path, script).expect("write fake staged binary");
    let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).expect("chmod fake staged binary");
    path
}

#[cfg(unix)]
#[tokio::test]
async fn probe_staged_binary_version_reads_self_reported_version() {
    let dir = tempdir().expect("tempdir");
    let staged = write_fake_staged_binary(dir.path(), "echo 'garyx 0.1.32'", 0);

    let measured = probe_staged_binary_version(&staged)
        .await
        .expect("probe should succeed");
    assert_eq!(measured, "0.1.32");
}

#[cfg(unix)]
#[tokio::test]
async fn probe_staged_binary_version_isolates_home() {
    // The probe must NOT touch the caller's real HOME. We point HOME at
    // a sentinel dir the fake binary is forbidden to write under (the
    // probe sets its own isolated HOME), then assert no `.garyx` shows
    // up there even though the fake binary tries to create one.
    let _guard = ENV_LOCK.lock().expect("env lock");
    let real_home = tempdir().expect("real home");
    let _home = ScopedEnvVar::set_path("HOME", real_home.path());

    let staged_dir = tempdir().expect("staged dir");
    let staged = write_fake_staged_binary(staged_dir.path(), "echo 'garyx 0.1.32'", 0);

    let measured = probe_staged_binary_version(&staged)
        .await
        .expect("probe should succeed");
    assert_eq!(measured, "0.1.32");

    // The fake binary creates `$HOME/.garyx/probe-marker`. If isolation
    // works, that landed in the probe's throwaway HOME (already dropped)
    // and NOT under the caller's HOME.
    assert!(
        !real_home.path().join(".garyx").exists(),
        "version probe leaked into the caller's HOME"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn probe_staged_binary_version_nonzero_exit_is_probe_failure() {
    let dir = tempdir().expect("tempdir");
    let staged = write_fake_staged_binary(dir.path(), "echo 'garyx 0.1.32'", 3);

    let err = probe_staged_binary_version(&staged)
        .await
        .expect_err("nonzero exit should fail the probe");
    assert!(matches!(err, SwapError::ProbeFailed { .. }), "got {err:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn probe_staged_binary_version_empty_stdout_is_probe_failure() {
    let dir = tempdir().expect("tempdir");
    // Exit 0 but print nothing usable.
    let staged = write_fake_staged_binary(dir.path(), "true", 0);

    let err = probe_staged_binary_version(&staged)
        .await
        .expect_err("empty stdout should fail the probe");
    assert!(matches!(err, SwapError::ProbeFailed { .. }), "got {err:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn probe_staged_binary_version_times_out() {
    // A binary that hangs must be killed by the timeout and surfaced as
    // a probe failure, never stalling the auto-update loop. We drive the
    // injectable-timeout inner fn with a tiny timeout so the test is
    // fast; the slow binary sleeps far longer than that.
    let dir = tempdir().expect("tempdir");
    let staged = write_fake_staged_binary(dir.path(), "sleep 30", 0);

    let err = probe_staged_binary_version_with_timeout(&staged, Duration::from_millis(150))
        .await
        .expect_err("hanging binary should fail the probe");
    assert!(matches!(err, SwapError::ProbeFailed { .. }), "got {err:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn probe_and_verify_removes_staged_file_on_version_mismatch() {
    // The staged binary is already `fs::copy`'d into the install dir by
    // the time verification runs. A bad release (tag advanced, binary
    // self-reports the old version) must NOT leave a `.garyx-update-*`
    // orphan behind — otherwise every auto-update retry tick leaks one.
    let dir = tempdir().expect("tempdir");
    let staged = write_fake_staged_binary(dir.path(), "echo 'garyx 0.1.29'", 0);
    assert!(staged.exists(), "fake staged binary should exist pre-verify");

    let err = probe_and_verify_staged_version(&staged, "0.1.32")
        .await
        .expect_err("version mismatch should be rejected");
    assert!(
        matches!(err, SwapError::VersionMismatch { .. }),
        "got {err:?}"
    );
    assert!(
        !staged.exists(),
        "staged temp file must be cleaned up on the reject path"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn probe_and_verify_removes_staged_file_on_probe_failure() {
    // Probe failures (nonzero exit here) take the same cleanup path.
    let dir = tempdir().expect("tempdir");
    let staged = write_fake_staged_binary(dir.path(), "echo 'garyx 0.1.32'", 3);
    assert!(staged.exists());

    let err = probe_and_verify_staged_version(&staged, "0.1.32")
        .await
        .expect_err("probe failure should be rejected");
    assert!(matches!(err, SwapError::ProbeFailed { .. }), "got {err:?}");
    assert!(
        !staged.exists(),
        "staged temp file must be cleaned up on probe failure"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn probe_and_verify_keeps_staged_file_on_success() {
    // On the happy path the staged file must SURVIVE so the caller can
    // `fs::rename` it into the install path.
    let dir = tempdir().expect("tempdir");
    let staged = write_fake_staged_binary(dir.path(), "echo 'garyx 0.1.32'", 0);

    let measured = probe_and_verify_staged_version(&staged, "0.1.32")
        .await
        .expect("matching version should pass");
    assert_eq!(measured, "0.1.32");
    assert!(
        staged.exists(),
        "staged temp file must remain for the caller to rename on success"
    );
}

#[test]
fn image_generation_prompt_preserves_user_prompt() {
    let user_prompt = "first line\nsecond line with [brackets]";
    let framed = build_image_generation_prompt(user_prompt);
    assert!(framed.contains("Generate exactly one image"));
    assert!(framed.contains("Do not merely describe an image"));
    assert!(framed.contains(user_prompt));
}

#[test]
fn tool_workspace_dir_uses_hidden_garyx_home_and_creates_directory() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let home = tempdir().expect("home");
    let _home = ScopedEnvVar::set_path("HOME", home.path());

    let workspace = tool_workspace_dir("search").expect("workspace");

    assert_eq!(
        workspace,
        home.path()
            .join(".garyx")
            .join("tool-workspaces")
            .join("search")
    );
    assert!(workspace.is_dir());
}

#[test]
fn tool_search_uses_fixed_gemini_flash_model() {
    assert_eq!(TOOL_SEARCH_GEMINI_MODEL, "gemini-3-flash-preview");
}

#[test]
fn gemini_search_policy_allows_only_google_web_search() {
    let policy = gemini_search_policy_text();
    assert!(policy.contains("toolName = \"*\""));
    assert!(policy.contains("decision = \"deny\""));
    assert!(policy.contains("toolName = \"google_web_search\""));
    assert!(policy.contains("decision = \"allow\""));
}

#[tokio::test]
async fn gemini_search_policy_is_temporary_and_removed_on_drop() {
    let policy = write_gemini_search_policy()
        .await
        .expect("temporary policy");
    let path = policy.path().to_owned();
    let dir = path.parent().expect("policy dir").to_owned();

    assert!(path.starts_with(std::env::temp_dir()));
    assert!(path.ends_with("search-tool-only-policy.toml"));
    assert_eq!(
        tokio::fs::read_to_string(&path).await.expect("policy text"),
        gemini_search_policy_text()
    );

    drop(policy);

    assert!(!path.exists(), "temporary policy file should be removed");
    assert!(
        !dir.exists(),
        "temporary policy directory should be removed"
    );
}

#[test]
fn gemini_cli_search_event_parser_requires_tool_use_not_direct_answer() {
    let mut state = SearchStreamState::default();
    let mut summary = GeminiCliSearchSummary::default();

    apply_gemini_cli_search_event(
        &mut state,
        &mut summary,
        &json!({
            "type": "init",
            "session_id": "session-1",
            "model": "gemini-3-flash-preview"
        }),
    );
    apply_gemini_cli_search_event(
        &mut state,
        &mut summary,
        &json!({
            "type": "message",
            "role": "assistant",
            "content": "Direct answer without a tool."
        }),
    );

    assert!(!state.searched);
    assert_eq!(state.answer, "Direct answer without a tool.");
    assert_eq!(summary.session_id.as_deref(), Some("session-1"));
    assert_eq!(summary.model.as_deref(), Some("gemini-3-flash-preview"));
}

#[test]
fn gemini_cli_search_event_parser_collects_tool_answer_and_stats() {
    let mut state = SearchStreamState::default();
    let mut summary = GeminiCliSearchSummary::default();

    apply_gemini_cli_search_event(
        &mut state,
        &mut summary,
        &json!({
            "type": "tool_use",
            "tool_name": "google_web_search",
            "tool_id": "google_web_search_1",
            "parameters": { "query": "example" }
        }),
    );
    apply_gemini_cli_search_event(
        &mut state,
        &mut summary,
        &json!({
            "type": "tool_result",
            "tool_id": "google_web_search_1",
            "status": "success",
            "output": "Search results returned."
        }),
    );
    apply_gemini_cli_search_event(
        &mut state,
        &mut summary,
        &json!({
            "type": "message",
            "role": "assistant",
            "content": "Answer with [Source](https://example.test/source)."
        }),
    );
    apply_gemini_cli_search_event(
        &mut state,
        &mut summary,
        &json!({
            "type": "result",
            "status": "success",
            "stats": {
                "duration_ms": 1234,
                "tool_calls": 1
            }
        }),
    );

    assert!(state.searched);
    assert_eq!(
        state.answer,
        "Answer with [Source](https://example.test/source)."
    );
    assert_eq!(summary.status.as_deref(), Some("success"));
    assert_eq!(summary.duration_ms, Some(1234));
    assert_eq!(state.tool_metadata.len(), 1);
    assert_eq!(state.tool_metadata[0].tool_name, "google_web_search");
    assert_eq!(
        state.tool_metadata[0].tool_use_id.as_deref(),
        Some("google_web_search_1")
    );
    assert_eq!(
        state.tool_metadata[0].output.as_deref(),
        Some("Search results returned.")
    );
}

#[test]
fn gemini_cli_stderr_sanitizer_redacts_sensitive_lines() {
    let stderr = "safe warning\nAuthorization: Bearer secret\nrefresh_token=secret";
    let sanitized = sanitize_gemini_cli_stderr(stderr);
    assert!(sanitized.contains("safe warning"));
    assert!(!sanitized.contains("Bearer secret"));
    assert!(!sanitized.contains("refresh_token=secret"));
    assert!(sanitized.contains("[redacted sensitive stderr line]"));
}

#[test]
fn search_stream_event_does_not_count_direct_answer_as_search() {
    let mut state = SearchStreamState::default();

    apply_search_stream_event(
        &mut state,
        &json!({
            "type": "committed_message",
            "thread_id": "thread::search",
            "run_id": "run-search",
            "seq": 1,
            "message": {
                "role": "assistant",
                "text": "I can answer this from memory without searching."
            }
        }),
    );

    assert!(!state.searched);
    assert_eq!(
        state.answer,
        "I can answer this from memory without searching."
    );
}

#[test]
fn extract_image_from_synthetic_tool_result_event() {
    let event = StreamEvent::ToolResult {
        message: ProviderMessage::tool_result(
            json!({
                "type": "imageGeneration",
                "id": "img_one",
                "media_type": "image/png",
                "result": "aGVsbG8="
            }),
            Some("img_one".to_owned()),
            Some("imageGeneration".to_owned()),
            Some(false),
        )
        .with_metadata_value("item_type", json!("imageGeneration")),
    };

    let image = extract_image_from_stream_event(&event)
        .expect("event parse")
        .expect("image");
    assert_eq!(image.bytes, b"hello");
    assert_eq!(image.extension, "png");
    assert_eq!(image.media_type.as_deref(), Some("image/png"));
}

#[test]
fn extract_image_from_synthetic_tool_result_event_rejects_malformed_base64() {
    let event = StreamEvent::ToolResult {
        message: ProviderMessage::tool_result(
            json!({
                "type": "imageGeneration",
                "id": "img_bad",
                "result": "not valid base64"
            }),
            Some("img_bad".to_owned()),
            Some("imageGeneration".to_owned()),
            Some(false),
        )
        .with_metadata_value("item_type", json!("imageGeneration")),
    };

    let error = extract_image_from_stream_event(&event).expect_err("malformed image");
    assert!(error.to_string().contains("malformed"));
}

#[test]
fn resolve_image_output_path_adds_extension_when_missing() {
    assert_eq!(
        resolve_image_output_path(PathBuf::from("/tmp/generated-image"), "webp"),
        PathBuf::from("/tmp/generated-image.webp")
    );
    assert_eq!(
        resolve_image_output_path(PathBuf::from("/tmp/generated-image.png"), "webp"),
        PathBuf::from("/tmp/generated-image.png")
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
        Some("worktree".to_owned()),
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
    assert_eq!(entry.workspace_mode.as_deref(), Some("worktree"));
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
fn upsert_channel_account_rejects_direct_workspace_mode() {
    let mut cfg = GaryxConfig::default();
    let err = upsert_channel_account(
        &mut cfg,
        "api",
        "scripted",
        None,
        None,
        Some("direct".to_owned()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Map::new(),
    )
    .expect_err("direct should not be accepted as a workspace mode");

    assert!(
        err.to_string().contains("use `local` or `worktree`"),
        "unexpected error: {err}"
    );
}

#[test]
fn validate_channel_account_configs_flags_null_plugin_config() {
    let mut cfg = GaryxConfig::default();
    cfg.channels
        .plugin_channel_mut("test-plugin")
        .accounts
        .insert(
            "test-account".to_owned(),
            PluginAccountEntry {
                enabled: true,
                name: Some("Test Account".to_owned()),
                agent_id: Some("claude".to_owned()),
                workspace_dir: None,
                workspace_mode: None,
                config: Value::Null,
            },
        );

    let issues = validate_channel_account_configs(&cfg, &std::collections::HashMap::new());

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, "CONFIG_CHANNEL_ACCOUNT_CONFIG_NULL");
    assert_eq!(
        issues[0].path.as_deref(),
        Some("$.channels.test-plugin.accounts.test-account.config")
    );
}

#[test]
fn validate_channel_account_configs_decodes_builtin_accounts() {
    let mut cfg = GaryxConfig::default();
    cfg.channels
        .plugin_channel_mut(BUILTIN_CHANNEL_PLUGIN_FEISHU)
        .accounts
        .insert(
            "work".to_owned(),
            PluginAccountEntry {
                enabled: true,
                name: None,
                agent_id: Some("claude".to_owned()),
                workspace_dir: None,
                workspace_mode: None,
                config: json!({}),
            },
        );

    let issues = validate_channel_account_configs(&cfg, &std::collections::HashMap::new());

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, "CONFIG_CHANNEL_ACCOUNT_INVALID");
    assert!(issues[0].message.contains("app_id"));
}

#[test]
fn validate_channel_account_configs_uses_installed_plugin_required_fields() {
    let mut cfg = GaryxConfig::default();
    cfg.channels
        .plugin_channel_mut("test-acmechat-cli")
        .accounts
        .insert(
            "agent-1".to_owned(),
            PluginAccountEntry {
                enabled: true,
                name: None,
                agent_id: Some("claude".to_owned()),
                workspace_dir: None,
                workspace_mode: None,
                config: json!({
                    "base_url": "https://chat.example.invalid"
                }),
            },
        );
    let schemas = std::collections::HashMap::from([(
        "test-acmechat-cli".to_owned(),
        json!({
            "type": "object",
            "required": ["token", "base_url"],
            "properties": {
                "token": { "type": "string" },
                "base_url": { "type": "string" }
            }
        }),
    )]);

    let issues = validate_channel_account_configs(&cfg, &schemas);

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, "CONFIG_CHANNEL_ACCOUNT_REQUIRED");
    assert!(issues[0].message.contains("token"));
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
fn task_notification_target_accepts_bot_and_none() {
    assert_eq!(
        task_notification_target_payload(vec!["none".to_owned()]).unwrap(),
        json!({ "kind": "none" })
    );
    assert_eq!(
        task_notification_target_payload(vec!["bot".to_owned(), "telegram:main".to_owned()])
            .unwrap(),
        json!({ "kind": "bot", "channel": "telegram", "account_id": "main" })
    );
}

#[test]
fn task_notification_target_resolves_current_thread_from_env() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _thread_id = ScopedEnvVar::set_string("GARYX_THREAD_ID", "thread::current");

    assert_eq!(
        task_notification_target_payload(vec!["current-thread".to_owned()]).unwrap(),
        json!({ "kind": "thread", "thread_id": "thread::current" })
    );
}

#[test]
fn task_source_payload_reads_runtime_env() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _thread_id = ScopedEnvVar::set_string("GARYX_THREAD_ID", "thread::current");
    let _task_id = ScopedEnvVar::set_string("GARYX_TASK_ID", "#TASK-7");
    let _bot_id = ScopedEnvVar::set_string("GARYX_BOT_ID", "telegram:main");
    let _channel = ScopedEnvVar::set_string("GARYX_CHANNEL", "telegram");
    let _account = ScopedEnvVar::set_string("GARYX_ACCOUNT_ID", "main");

    assert_eq!(
        task_source_payload_from_env().unwrap(),
        json!({
            "thread_id": "thread::current",
            "task_id": "#TASK-7",
            "task_thread_id": "thread::current",
            "bot_id": "telegram:main",
            "channel": "telegram",
            "account_id": "main",
        })
    );
}

#[test]
fn task_id_display_falls_back_to_task_number() {
    let payload = json!({
        "task": {
            "number": 42,
            "title": "Fallback ref"
        }
    });

    assert_eq!(task_id_display(&payload, &payload["task"]), "#TASK-42");
}

#[test]
fn format_task_progress_groups_each_user_turn_with_last_assistant_text_group() {
    let task_payload = json!({
        "task_id": "#TASK-42",
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
fn append_task_workflow_run_renders_run_and_child_summary() {
    let mut output = String::from("Task: #TASK-42\n");
    let workflow_run = json!({
        "workflow": {
            "workflowRunId": "run-abc",
            "workflowId": "run-abc",
            "status": "succeeded",
            "workflowDefinitionId": "deep-research",
            "workflowDefinitionVersion": 2,
            "totalChildren": 2,
            "completedChildren": 2,
            "failedChildren": 0,
            "outputText": "Done"
        },
        "children": [
            {
                "label": "Search",
                "status": "succeeded",
                "phaseTitle": "Search",
                "threadId": "thread::child"
            }
        ],
        "events": []
    });

    append_task_workflow_run(&mut output, Some(&workflow_run));

    assert!(output.contains("Workflow Run:"));
    assert!(
        output.contains("- run-abc [succeeded] definition deep-research@2 children 2/2 failed 0")
    );
    assert!(output.contains("Output: Done"));
    assert!(output.contains("- Search [succeeded] phase Search thread thread::child"));
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

fn agent_value(agent_id: &str, built_in: bool) -> Value {
    json!({
        "agent_id": agent_id,
        "display_name": agent_id,
        "provider_type": "claude_code",
        "built_in": built_in,
    })
}

#[test]
fn sort_agents_builtin_first_groups_builtins_then_alphabetical() {
    let mut agents = vec![
        agent_value("novelist", false),
        agent_value("codex", true),
        agent_value("gary", false),
        agent_value("claude", true),
        agent_value("gemini", true),
    ];

    sort_agents_builtin_first(&mut agents);

    let order: Vec<&str> = agents
        .iter()
        .map(|a| a["agent_id"].as_str().unwrap())
        .collect();
    assert_eq!(order, vec!["claude", "codex", "gemini", "gary", "novelist"]);
}

#[test]
fn sort_agents_builtin_first_treats_missing_flag_as_custom() {
    let mut agents = vec![
        json!({ "agent_id": "zeta" }),
        agent_value("alpha", true),
        json!({ "agent_id": "alpha-custom" }),
    ];

    sort_agents_builtin_first(&mut agents);

    let order: Vec<&str> = agents
        .iter()
        .map(|a| a["agent_id"].as_str().unwrap())
        .collect();
    assert_eq!(order, vec!["alpha", "alpha-custom", "zeta"]);
}

#[test]
fn sort_agents_builtin_first_handles_empty_slice() {
    let mut agents: Vec<Value> = Vec::new();
    sort_agents_builtin_first(&mut agents);
    assert!(agents.is_empty());
}

#[test]
fn decorate_agent_list_json_adds_kind_and_sorts_in_place() {
    let payload = json!({
        "agents": [
            { "agent_id": "novelist", "display_name": "Novelist", "built_in": false, "model": "gpt-5" },
            { "agent_id": "codex", "display_name": "Codex", "built_in": true },
            { "agent_id": "gary", "display_name": "Gary", "built_in": false },
            { "agent_id": "claude", "display_name": "Claude", "built_in": true },
        ],
    });

    let decorated = decorate_agent_list_json(payload);

    let agents = decorated["agents"].as_array().expect("agents array");
    let order: Vec<&str> = agents
        .iter()
        .map(|a| a["agent_id"].as_str().unwrap())
        .collect();
    assert_eq!(order, vec!["claude", "codex", "gary", "novelist"]);

    assert_eq!(agents[0]["kind"], "builtin");
    assert_eq!(agents[1]["kind"], "builtin");
    assert_eq!(agents[2]["kind"], "custom");
    assert_eq!(agents[3]["kind"], "custom");

    // Original fields survive untouched.
    assert_eq!(agents[0]["display_name"], "Claude");
    assert_eq!(agents[0]["built_in"], true);
    assert_eq!(agents[3]["model"], "gpt-5");
    assert_eq!(agents[3]["built_in"], false);
}

#[test]
fn decorate_agent_list_json_preserves_top_level_shape_when_agents_missing() {
    let payload = json!({ "other": "value" });
    let decorated = decorate_agent_list_json(payload);
    assert_eq!(decorated, json!({ "other": "value" }));
}

#[test]
fn decorate_agent_list_json_handles_empty_array() {
    let payload = json!({ "agents": [] });
    let decorated = decorate_agent_list_json(payload);
    assert_eq!(decorated, json!({ "agents": [] }));
}

#[test]
fn gui_session_available_false_when_both_unset() {
    assert!(!gui_session_available(None, None));
}

#[test]
fn gui_session_available_false_when_both_empty() {
    // X11 convention: an empty DISPLAY behaves like unset, and
    // xdg-open's fallback chain treats it the same way.
    assert!(!gui_session_available(
        Some(OsStr::new("")),
        Some(OsStr::new(""))
    ));
}

#[test]
fn gui_session_available_true_with_x11_display() {
    assert!(gui_session_available(Some(OsStr::new(":0")), None));
}

#[test]
fn gui_session_available_true_with_wayland_only() {
    assert!(gui_session_available(None, Some(OsStr::new("wayland-0"))));
}

#[test]
fn blocked_transition_in_review_to_in_progress_points_to_send_message() {
    let message = blocked_task_status_transition("in_review", "in_progress", "#TASK-12")
        .expect("in_review -> in_progress must be blocked");
    assert!(message.contains("In Review"));
    assert!(message.contains("In Progress"));
    // The guidance must hand the user a copy-pasteable send-message command;
    // the task id is single-quoted so the shell does not treat the leading `#`
    // in canonical `#TASK-*` ids as a comment.
    assert!(message.contains("garyx thread send task '#TASK-12'"));
}

#[test]
fn blocked_transition_in_progress_to_in_review_explains_it_is_automatic() {
    let message = blocked_task_status_transition("in_progress", "in_review", "#TASK-12")
        .expect("in_progress -> in_review must be blocked");
    assert!(message.contains("automatically"));
    assert!(message.contains("cannot be set manually"));
}

#[test]
fn allowed_transitions_are_not_blocked() {
    // The one allowed move out of review, plus the ordinary start/stop/reopen
    // transitions, must all pass through to the gateway untouched.
    for (from, to) in [
        ("in_review", "done"),
        ("todo", "in_progress"),
        ("in_progress", "todo"),
        ("done", "todo"),
    ] {
        assert!(
            blocked_task_status_transition(from, to, "#TASK-1").is_none(),
            "{from} -> {to} should be allowed"
        );
    }
}

#[test]
fn current_task_status_reads_nested_then_top_level() {
    assert_eq!(
        current_task_status(&json!({ "task": { "status": "in_review" } })),
        Some("in_review")
    );
    assert_eq!(
        current_task_status(&json!({ "status": "in_progress" })),
        Some("in_progress")
    );
    assert_eq!(
        current_task_status(&json!({ "thread_id": "thread::x" })),
        None
    );
}

/// Mock gateway serving `GET /api/tasks/{id}` with a fixed status and recording
/// every lookup, so tests can assert both the decision and whether the status
/// lookup was issued at all.
async fn spawn_task_get_server(
    status: &'static str,
    requests: StdArc<Mutex<Vec<RecordedRequest>>>,
) -> (String, JoinHandle<()>) {
    let app = Router::new().route(
        "/api/tasks/{task_id}",
        get(move |AxumPath(task_id): AxumPath<String>| {
            let requests = requests.clone();
            async move {
                requests
                    .lock()
                    .expect("request lock")
                    .push(RecordedRequest {
                        method: "GET".to_owned(),
                        path: format!("/api/tasks/{task_id}"),
                        body: Value::Null,
                    });
                Json(json!({ "task": { "status": status } }))
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
async fn blocked_status_update_refuses_review_to_progress_after_one_lookup() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_task_get_server("in_review", requests.clone()).await;
    let gateway = GatewayEndpoint {
        base_url,
        auth_token: None,
    };

    let blocked = blocked_status_update(&gateway, "#TASK-7", "in_progress", false)
        .await
        .expect("status lookup should succeed");

    handle.abort();
    let message = blocked.expect("in_review -> in_progress must be blocked");
    assert!(message.contains("garyx thread send task '#TASK-7'"));
    assert_eq!(
        requests.lock().expect("request lock").len(),
        1,
        "should look up current status exactly once"
    );
}

#[tokio::test]
async fn blocked_status_update_refuses_progress_to_review() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_task_get_server("in_progress", requests.clone()).await;
    let gateway = GatewayEndpoint {
        base_url,
        auth_token: None,
    };

    let blocked = blocked_status_update(&gateway, "#TASK-7", "in_review", false)
        .await
        .expect("status lookup should succeed");

    handle.abort();
    assert!(
        blocked
            .expect("in_progress -> in_review must be blocked")
            .contains("automatically")
    );
}

#[tokio::test]
async fn blocked_status_update_allows_todo_to_progress() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_task_get_server("todo", requests.clone()).await;
    let gateway = GatewayEndpoint {
        base_url,
        auth_token: None,
    };

    let blocked = blocked_status_update(&gateway, "#TASK-7", "in_progress", false)
        .await
        .expect("status lookup should succeed");

    handle.abort();
    assert!(blocked.is_none(), "starting a todo task must be allowed");
    assert_eq!(requests.lock().expect("request lock").len(), 1);
}

#[tokio::test]
async fn blocked_status_update_skips_lookup_when_completing() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_task_get_server("in_review", requests.clone()).await;
    let gateway = GatewayEndpoint {
        base_url,
        auth_token: None,
    };

    // Completing a reviewed task is the allowed move and must not be gated, so
    // it should never issue the current-status lookup.
    let blocked = blocked_status_update(&gateway, "#TASK-7", "done", false)
        .await
        .expect("done update should not error");

    handle.abort();
    assert!(blocked.is_none());
    assert!(
        requests.lock().expect("request lock").is_empty(),
        "completing a task should not look up current status"
    );
}

#[tokio::test]
async fn blocked_status_update_force_overrides_guard_without_lookup() {
    let requests = StdArc::new(Mutex::new(Vec::new()));
    let (base_url, handle) = spawn_task_get_server("in_review", requests.clone()).await;
    let gateway = GatewayEndpoint {
        base_url,
        auth_token: None,
    };

    // --force is an explicit override: even the otherwise-blocked
    // in_review -> in_progress move is allowed through, and the guard does not
    // even look up the current status.
    let blocked = blocked_status_update(&gateway, "#TASK-7", "in_progress", true)
        .await
        .expect("forced update should not error");

    handle.abort();
    assert!(blocked.is_none(), "--force must override the guard");
    assert!(
        requests.lock().expect("request lock").is_empty(),
        "a forced update should not look up current status"
    );
}
