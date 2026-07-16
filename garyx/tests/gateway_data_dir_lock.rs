#![cfg(unix)]

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::json;
use tempfile::tempdir;
use tokio::process::{Child, Command};

fn reserve_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
    listener.local_addr().unwrap().port()
}

fn write_config(path: &Path, port: u16, data_dir: Option<&Path>) {
    let mut config = json!({
        "gateway": {"host": "127.0.0.1", "port": port}
    });
    if let Some(data_dir) = data_dir {
        config["sessions"] = json!({"data_dir": data_dir});
    }
    std::fs::write(path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
}

async fn spawn_gateway(config: &Path, home: &Path, force_subprocess_restart: bool) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_garyx"));
    command
        .args([
            "--config",
            config.to_str().unwrap(),
            "gateway",
            "run",
            "--no-channels",
        ])
        .env("HOME", home)
        .env("GARYX_DATA_LOCK_WAIT_SECS", "5")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if force_subprocess_restart {
        command.env("GARYX_TEST_FORCE_SUBPROCESS_RESTART", "1");
    }
    command.spawn().expect("spawn gateway")
}

async fn wait_for_health(port: u16) {
    let url = format!("http://127.0.0.1:{port}/health");
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if reqwest::get(&url)
                .await
                .is_ok_and(|response| response.status().is_success())
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("gateway health timeout");
}

async fn kill_pid(pid: u32) {
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .await;
}

async fn stop_child(child: &mut Child) {
    if let Some(pid) = child.id() {
        kill_pid(pid).await;
    }
    let _ = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
}

async fn wait_for_lock_pid(lock_path: &Path, not_pid: Option<u32>) -> u32 {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(raw) = std::fs::read_to_string(lock_path)
                && let Ok(pid) = raw.trim().parse::<u32>()
                && Some(pid) != not_pid
            {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("lock owner PID timeout")
}

#[tokio::test]
async fn real_runtime_assembler_uses_exactly_the_selected_default_or_custom_database() {
    let temp = tempdir().expect("temp dir");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();

    let custom_data = temp.path().join("custom-data");
    let custom_config = temp.path().join("custom.json");
    let custom_port = reserve_port();
    write_config(&custom_config, custom_port, Some(&custom_data));
    let mut custom_gateway = spawn_gateway(&custom_config, &home, false).await;
    wait_for_health(custom_port).await;
    assert!(custom_data.join("garyx-db.sqlite3").exists());
    assert!(custom_data.join("garyx.lock").exists());
    assert!(
        !home.join(".garyx/data/garyx-db.sqlite3").exists(),
        "custom data-dir assembly must not initialize the default database"
    );
    stop_child(&mut custom_gateway).await;

    let default_config = temp.path().join("default.json");
    let default_port = reserve_port();
    write_config(&default_config, default_port, None);
    let mut default_gateway = spawn_gateway(&default_config, &home, false).await;
    wait_for_health(default_port).await;
    let default_data = home.join(".garyx/data");
    assert!(default_data.join("garyx-db.sqlite3").exists());
    assert!(default_data.join("garyx.lock").exists());
    stop_child(&mut default_gateway).await;
}

#[tokio::test]
async fn unmanaged_subprocess_restart_waits_for_lock_then_serves_with_a_new_owner() {
    let temp = tempdir().expect("temp dir");
    let home = temp.path().join("home");
    let data_dir = temp.path().join("restart-data");
    std::fs::create_dir_all(&home).unwrap();
    let config_path = temp.path().join("garyx.json");
    let port = reserve_port();
    write_config(&config_path, port, Some(&data_dir));

    let mut old_gateway = spawn_gateway(&config_path, &home, true).await;
    wait_for_health(port).await;
    let old_pid = old_gateway.id().expect("old gateway pid");
    assert_eq!(
        wait_for_lock_pid(&data_dir.join("garyx.lock"), None).await,
        old_pid
    );

    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/restart"))
        .json(&json!({}))
        .send()
        .await
        .expect("request unmanaged restart");
    assert!(response.status().is_success());
    tokio::time::timeout(Duration::from_secs(10), old_gateway.wait())
        .await
        .expect("old gateway exit timeout")
        .expect("old gateway wait");

    let new_pid = wait_for_lock_pid(&data_dir.join("garyx.lock"), Some(old_pid)).await;
    wait_for_health(port).await;
    assert_ne!(new_pid, old_pid);
    kill_pid(new_pid).await;
}
