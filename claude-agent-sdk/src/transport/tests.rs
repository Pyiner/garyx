use super::*;
use crate::types::ClaudeAgentOptions;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

#[test]
fn test_subprocess_transport_creation() {
    let opts = ClaudeAgentOptions::default();
    let transport = SubprocessTransport::new(opts, true);
    assert!(!transport.is_ready());
    assert!(transport.streaming);
}

#[test]
fn test_build_command_streaming() {
    let opts = ClaudeAgentOptions {
        model: Some("claude-sonnet-4-5".into()),
        max_turns: Some(5),
        ..Default::default()
    };
    let transport = SubprocessTransport::new(opts, true);
    let cmd = transport.build_command(None);
    let args: Vec<_> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();

    assert!(args.contains(&"--output-format".to_string()));
    assert!(args.contains(&"stream-json".to_string()));
    assert!(args.contains(&"--verbose".to_string()));
    assert!(args.contains(&"--print".to_string()));
    assert!(args.contains(&"--model".to_string()));
    assert!(args.contains(&"claude-sonnet-4-5".to_string()));
    assert!(args.contains(&"--max-turns".to_string()));
    assert!(args.contains(&"5".to_string()));
    assert!(args.contains(&"--input-format".to_string()));
}

#[test]
fn test_build_command_overlays_configured_env() {
    // Deterministic proof that agent-configured env reaches the spawned
    // `claude` CLI Command, without starting a real process.
    let mut env = HashMap::new();
    env.insert("TEST_AGENT_ENV_KEY".to_string(), "test-value".to_string());
    let opts = ClaudeAgentOptions {
        env,
        ..Default::default()
    };
    let transport = SubprocessTransport::new(opts, true);
    let cmd = transport.build_command(None);
    let std_cmd = cmd.as_std();

    let has_env = std_cmd.get_envs().any(|(key, value)| {
        key == std::ffi::OsStr::new("TEST_AGENT_ENV_KEY")
            && value == Some(std::ffi::OsStr::new("test-value"))
    });
    assert!(has_env, "configured agent env var must reach the spawn Command");

    // The env value must not leak into program/args (no-proactive-leak).
    assert!(!std_cmd.get_program().to_string_lossy().contains("test-value"));
    assert!(
        std_cmd
            .get_args()
            .all(|arg| !arg.to_string_lossy().contains("test-value"))
    );
}

#[test]
fn test_build_command_inserts_cli_prefix_args_before_sdk_args() {
    let opts = ClaudeAgentOptions {
        cli_prefix_args: vec!["__cctty".into()],
        model: Some("claude-sonnet-4-5".into()),
        ..Default::default()
    };
    let transport = SubprocessTransport::new(opts, true);
    let cmd = transport.build_command(None);
    let args: Vec<_> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();

    assert_eq!(args.first().map(String::as_str), Some("__cctty"));
    assert!(args.contains(&"--model".to_string()));
}

#[test]
fn test_build_command_oneshot() {
    let opts = ClaudeAgentOptions::default();
    let transport = SubprocessTransport::new(opts, false);
    let cmd = transport.build_command(Some("hello world"));
    let args: Vec<_> = cmd
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();

    assert!(args.contains(&"--print".to_string()));
    assert!(args.contains(&"--".to_string()));
    assert!(args.contains(&"hello world".to_string()));
    assert!(!args.contains(&"--input-format".to_string()));
}

#[test]
fn test_resolve_cli_path_prefers_candidate_with_required_flags() {
    let fixture = CliFixture::new(&[
        ("old/bin/claude", "Usage: claude [options]\n  --print\n"),
        (
            "new/bin/claude",
            "Usage: claude [options]\n  --print\n  --agent <agent>\n  --agents <json>\n",
        ),
    ]);

    let mut opts = ClaudeAgentOptions {
        agent: Some("claude".into()),
        agents: HashMap::from([(
            "claude".into(),
            crate::types::ClaudeAgentDefinition {
                description: "Builtin Claude".into(),
                prompt: "You are Claude".into(),
            },
        )]),
        ..Default::default()
    };
    opts.env
        .insert("PATH".into(), fixture.path_env(&["old/bin", "new/bin"]));
    opts.env
        .insert("HOME".into(), fixture.root.to_string_lossy().to_string());

    let cli_path = resolve_cli_path(&opts);
    assert_eq!(cli_path, fixture.absolute("new/bin/claude"));
}

#[test]
fn test_resolve_cli_path_falls_back_to_first_candidate_without_required_flags() {
    let fixture = CliFixture::new(&[("only/bin/claude", "Usage: claude [options]\n  --print\n")]);

    let mut opts = ClaudeAgentOptions {
        agents: HashMap::from([(
            "claude".into(),
            crate::types::ClaudeAgentDefinition {
                description: "Builtin Claude".into(),
                prompt: "You are Claude".into(),
            },
        )]),
        ..Default::default()
    };
    opts.env
        .insert("PATH".into(), fixture.path_env(&["only/bin"]));
    opts.env
        .insert("HOME".into(), fixture.root.to_string_lossy().to_string());

    let cli_path = resolve_cli_path(&opts);
    assert_eq!(cli_path, fixture.absolute("only/bin/claude"));
}

#[tokio::test]
async fn test_close_terminates_process_before_waiting_for_reader_lock() {
    let fixture = CliFixture::new_raw(&[(
        "bin/claude",
        "#!/bin/sh\nIFS= read -r line || true\nexec sleep 600\n",
    )]);
    let opts = ClaudeAgentOptions {
        cli_path: Some(fixture.root.join("bin/claude")),
        ..ClaudeAgentOptions::default()
    };
    let transport = Arc::new(SubprocessTransport::new(opts, true));
    transport.spawn(None).await.unwrap();

    let reader_guard = transport.reader.lock().await;
    let close_transport = Arc::clone(&transport);
    let close_handle = tokio::spawn(async move { close_transport.close().await });

    tokio::time::sleep(CLOSE_GRACE_TIMEOUT + Duration::from_millis(200)).await;
    drop(reader_guard);

    let result = tokio::time::timeout(Duration::from_secs(1), close_handle)
        .await
        .expect("close should have already killed the process before waiting for reader cleanup")
        .expect("close task should not panic");
    result.expect("close should succeed");
}

struct CliFixture {
    root: PathBuf,
}

impl CliFixture {
    fn new(entries: &[(&str, &str)]) -> Self {
        let root = std::env::temp_dir().join(format!("claude-cli-fixture-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();

        for (relative_path, help_text) in entries {
            let script_path = root.join(relative_path);
            if let Some(parent) = script_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            write_cli_script(&script_path, help_text);
        }

        Self { root }
    }

    fn new_raw(entries: &[(&str, &str)]) -> Self {
        let root = std::env::temp_dir().join(format!("claude-cli-fixture-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();

        for (relative_path, script) in entries {
            let script_path = root.join(relative_path);
            if let Some(parent) = script_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            write_raw_script(&script_path, script);
        }

        Self { root }
    }

    fn absolute(&self, relative_path: &str) -> String {
        fs::canonicalize(self.root.join(relative_path))
            .unwrap()
            .to_string_lossy()
            .to_string()
    }

    fn path_env(&self, entries: &[&str]) -> String {
        std::env::join_paths(entries.iter().map(|entry| self.root.join(entry)))
            .unwrap()
            .to_string_lossy()
            .to_string()
    }
}

impl Drop for CliFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_cli_script(path: &Path, help_text: &str) {
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"--help\" ]; then\ncat <<'EOF'\n{help_text}EOF\nexit 0\nfi\nprintf '%s\\n' \"$@\"\n"
    );
    fs::write(path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }
}

fn write_raw_script(path: &Path, script: &str) {
    fs::write(path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }
}
