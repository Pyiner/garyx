#![cfg(unix)]

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

static CCTTY_ENV_LOCK: Mutex<()> = Mutex::new(());

#[tokio::test]
async fn cctty_normal_completion_preserves_user_background_process() {
    let _guard = CCTTY_ENV_LOCK.lock().expect("cctty env lock poisoned");
    let fixture = CcttyCleanupFixture::new();
    let original_dir = std::env::current_dir().expect("current dir");
    let pid_guard = BackgroundPidGuard {
        pid_path: fixture.background_pid_path.clone(),
    };

    std::env::set_current_dir(fixture.workspace.path()).expect("set temp workspace dir");
    unsafe {
        std::env::set_var("CCTTY_CLAUDE_PATH", &fixture.fake_claude_path);
        std::env::set_var("CLAUDE_CONFIG_DIR", fixture.claude_config.path());
        std::env::set_var(
            "FAKE_CLAUDE_BACKGROUND_PID_PATH",
            &fixture.background_pid_path,
        );
    }

    let exit_code = cctty::run_cli(vec![
        "cctty".to_owned(),
        "--print".to_owned(),
        "--".to_owned(),
        "START_BACKGROUND_SERVICE".to_owned(),
    ])
    .await
    .expect("cctty run should complete");

    unsafe {
        std::env::remove_var("CCTTY_CLAUDE_PATH");
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        std::env::remove_var("FAKE_CLAUDE_BACKGROUND_PID_PATH");
    }
    std::env::set_current_dir(original_dir).expect("restore current dir");

    assert_eq!(exit_code, 0);

    let pid = pid_guard.pid();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(
        process_exists(pid),
        "user-started background process {pid} should survive normal cctty completion"
    );
}

struct CcttyCleanupFixture {
    workspace: tempfile::TempDir,
    claude_config: tempfile::TempDir,
    fake_claude_path: std::path::PathBuf,
    background_pid_path: std::path::PathBuf,
}

impl CcttyCleanupFixture {
    fn new() -> Self {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let claude_config = tempfile::tempdir().expect("claude config tempdir");
        let fake_claude_path = workspace.path().join("fake-claude");
        let background_pid_path = workspace.path().join("background.pid");
        write_fake_claude(&fake_claude_path);
        Self {
            workspace,
            claude_config,
            fake_claude_path,
            background_pid_path,
        }
    }
}

struct BackgroundPidGuard {
    pid_path: std::path::PathBuf,
}

impl BackgroundPidGuard {
    fn pid(&self) -> u32 {
        fs::read_to_string(&self.pid_path)
            .expect("background pid file should exist")
            .trim()
            .parse()
            .expect("background pid should be numeric")
    }
}

impl Drop for BackgroundPidGuard {
    fn drop(&mut self) {
        let Ok(raw_pid) = fs::read_to_string(&self.pid_path) else {
            return;
        };
        let Ok(pid) = raw_pid.trim().parse::<u32>() else {
            return;
        };
        if process_exists(pid) {
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .status();
            std::thread::sleep(Duration::from_millis(50));
        }
        if process_exists(pid) {
            let _ = Command::new("kill")
                .arg("-KILL")
                .arg(pid.to_string())
                .status();
        }
    }
}

fn write_fake_claude(path: &Path) {
    fs::write(
        path,
        r#"#!/usr/bin/env python3
import json
import os
from pathlib import Path
import subprocess
import sys

if "--help" in sys.argv or "-h" in sys.argv:
    print("Usage: claude [options] [prompt]")
    print("  --print")
    print("  --input-format <format>")
    print("  --output-format <format>")
    sys.exit(0)

if "--version" in sys.argv or "-v" in sys.argv:
    print("fake claude 0.0.0")
    sys.exit(0)

def arg_value(flag, default=None):
    if flag in sys.argv:
        index = sys.argv.index(flag)
        if index + 1 < len(sys.argv):
            return sys.argv[index + 1]
    for arg in sys.argv:
        if arg.startswith(flag + "="):
            return arg.split("=", 1)[1]
    return default

def project_key(cwd):
    out = []
    for ch in str(cwd):
        out.append(ch if ch.isascii() and ch.isalnum() else "-")
    return "".join(out) or "-"

session_id = arg_value("--session-id") or arg_value("--resume") or "00000000-0000-0000-0000-000000000000"
config_dir = Path(os.environ.get("CLAUDE_CONFIG_DIR", str(Path.home() / ".claude")))
transcript = config_dir / "projects" / project_key(Path.cwd()) / f"{session_id}.jsonl"
transcript.parent.mkdir(parents=True, exist_ok=True)

def ready():
    sys.stdout.write("Context permissions /mcp\n")
    sys.stdout.write("❯ \n")
    sys.stdout.flush()

ready()
buffer = b""
while True:
    chunk = os.read(0, 4096)
    if not chunk:
        break
    buffer += chunk
    end = buffer.find(b"\x1b[201~")
    if end < 0:
        continue
    start = buffer.find(b"\x1b[200~")
    raw_prompt = buffer[start + len(b"\x1b[200~"):end] if start >= 0 else buffer[:end]
    prompt = raw_prompt.decode("utf-8", errors="replace")
    response = "FAKE_RESPONSE: " + prompt
    if "START_BACKGROUND_SERVICE" in prompt:
        child = subprocess.Popen(
            ["sh", "-c", "trap '' HUP; exec sleep 600"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        Path(os.environ["FAKE_CLAUDE_BACKGROUND_PID_PATH"]).write_text(str(child.pid), encoding="utf-8")
    with transcript.open("a", encoding="utf-8") as f:
        f.write(json.dumps({"type":"system","subtype":"init","session_id":session_id}) + "\n")
        f.write(json.dumps({"type":"user","message":{"role":"user","content":prompt}}) + "\n")
        f.write(json.dumps({"type":"assistant","message":{"model":"fake-model","content":[{"type":"text","text":response}]}}) + "\n")
        f.write(json.dumps({"type":"result","subtype":"success","duration_ms":1,"duration_api_ms":1,"is_error":False,"num_turns":1,"session_id":session_id,"result":response,"usage":{"input_tokens":1,"output_tokens":1}}) + "\n")
    ready()
    after = end + len(b"\x1b[201~")
    while after < len(buffer) and buffer[after:after + 1] in (b"\r", b"\n"):
        after += 1
    buffer = buffer[after:]
"#,
    )
    .expect("write fake claude");

    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)
        .expect("fake claude metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod fake claude");
}

fn process_exists(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
