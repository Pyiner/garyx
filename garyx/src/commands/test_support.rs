use super::*;
use axum::{Json, Router, http::StatusCode, routing::post};
use std::ffi::OsStr;
use std::ffi::OsString;
use std::sync::{Arc as StdArc, Mutex};
use tokio::{net::TcpListener, task::JoinHandle};

pub(super) static ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
pub(super) struct RecordedRequest {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) body: Value,
}

pub(super) struct ScopedEnvVar {
    pub(super) key: &'static str,
    pub(super) previous: Option<OsString>,
}

impl ScopedEnvVar {
    pub(super) fn set_path(key: &'static str, value: &Path) -> Self {
        Self::set_value(key, value.as_os_str())
    }

    pub(super) fn set_string(key: &'static str, value: &str) -> Self {
        Self::set_value(key, OsStr::new(value))
    }

    pub(super) fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::remove_var(key);
        }
        Self { key, previous }
    }

    pub(super) fn set_value(key: &'static str, value: &OsStr) -> Self {
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

pub(super) fn write_test_gateway_config(
    dir: &tempfile::TempDir,
    public_url: &str,
) -> std::path::PathBuf {
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

pub(super) async fn spawn_thread_task_http_test_server(
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

pub(super) async fn spawn_disabled_agent_rejection_server(
    requests: StdArc<Mutex<Vec<RecordedRequest>>>,
) -> (String, JoinHandle<()>) {
    let thread_requests = requests.clone();
    let task_requests = requests;
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
                            body: payload,
                        });
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": "agent is disabled: codex"})),
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
                            body: payload,
                        });
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": "agent is disabled: codex"})),
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
