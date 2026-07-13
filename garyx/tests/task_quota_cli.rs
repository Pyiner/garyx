use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use axum::{Json, Router, extract::State, http::StatusCode, routing::get, routing::post};
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::{net::TcpListener, process::Command, task::JoinHandle};

#[derive(Clone)]
struct TestState {
    task_posts: Arc<AtomicUsize>,
}

async fn spawn_exhausted_gateway() -> (String, Arc<AtomicUsize>, JoinHandle<()>) {
    let task_posts = Arc::new(AtomicUsize::new(0));
    let state = TestState {
        task_posts: task_posts.clone(),
    };
    let app = Router::new()
        .route(
            "/api/custom-agents/{agent_id}",
            get(|| async {
                Json(json!({
                    "agent_id": "test-agent",
                    "display_name": "Test Agent",
                    "provider_type": "codex_app_server",
                    "model": "test-model",
                    "provider_env": {},
                    "system_prompt": "",
                    "built_in": false,
                    "standalone": true,
                    "created_at": "2030-01-01T00:00:00Z",
                    "updated_at": "2030-01-01T00:00:00Z"
                }))
            }),
        )
        .route(
            "/api/usage/coding",
            get(|| async {
                Json(json!({
                    "providers": [{
                        "id": "codex",
                        "available": true,
                        "session": {
                            "used_percent": 100.0,
                            "remaining_percent": 0.0,
                            "resets_at": "2030-01-02T12:00:00Z"
                        }
                    }],
                    "refreshed_at": "2030-01-01T12:00:00Z"
                }))
            }),
        )
        .route(
            "/api/tasks",
            post(
                |State(state): State<TestState>, Json(_payload): Json<Value>| async move {
                    state.task_posts.fetch_add(1, Ordering::SeqCst);
                    (
                        StatusCode::CREATED,
                        Json(json!({"task_id": "#TASK-1000000001"})),
                    )
                },
            ),
        )
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("listener addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test router");
    });
    (format!("http://{addr}"), task_posts, handle)
}

#[tokio::test]
async fn json_exhaustion_is_typed_exit_one_and_never_posts_task() {
    let (base_url, task_posts, handle) = spawn_exhausted_gateway().await;
    let dir = tempdir().expect("tempdir");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).expect("test home");
    let config_path = dir.path().join("garyx.json");
    std::fs::write(
        &config_path,
        serde_json::to_vec(&json!({
            "gateway": {"public_url": base_url}
        }))
        .expect("config json"),
    )
    .expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_garyx"))
        .args([
            "--config",
            config_path.to_str().expect("config path"),
            "task",
            "create",
            "--title",
            "Synthetic quota task",
            "--agent",
            "test-agent",
            "--notify",
            "none",
            "--json",
        ])
        .env("HOME", &home)
        .env_remove("GARYX_THREAD_ID")
        .env_remove("GARYX_TASK_ID")
        .env_remove("GARYX_CHANNEL")
        .env_remove("GARYX_ACCOUNT_ID")
        .env_remove("GARYX_BOT_ID")
        .output()
        .await
        .expect("run garyx");
    handle.abort();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(task_posts.load(Ordering::SeqCst), 0);
    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    let stderr = String::from_utf8(output.stderr).expect("stderr utf8");
    let envelope: Value = serde_json::from_str(&stdout).expect("one JSON failure envelope");
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["kind"], "provider_quota_exhausted");
    assert!(
        envelope["error"]["message"]
            .as_str()
            .expect("message")
            .contains("task was not created")
    );
    assert!(stdout.find("Warning:").is_none());
    assert!(stderr.find("Warning:").is_none());
}
