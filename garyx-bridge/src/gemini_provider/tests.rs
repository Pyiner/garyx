use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc as StdArc, Mutex as StdMutex};

#[test]
fn build_mcp_servers_injects_builtin_and_preserves_remote_shapes() {
    let config = GeminiCliConfig {
        mcp_base_url: "http://127.0.0.1:31337".to_owned(),
        ..Default::default()
    };
    let metadata = HashMap::from([(
        "remote_mcp_servers".to_owned(),
        json!({
            "proof-http": {
                "url": "http://127.0.0.1:9000/mcp",
                "headers": { "Authorization": "Bearer demo" }
            },
            "proof-stdio": {
                "command": "python3",
                "args": ["server.py"],
                "env": { "TOKEN": "demo" }
            }
        }),
    )]);

    let servers = build_mcp_servers(&config, "thread::1", "run-1", &metadata);
    assert_eq!(servers.len(), 3);
    assert!(servers.iter().any(|server| server["name"] == "garyx"));
    assert!(
        servers
            .iter()
            .any(|server| server["name"] == "proof-http" && server["type"] == "http")
    );
    assert!(
        servers
            .iter()
            .any(|server| server["name"] == "proof-stdio" && server["command"] == "python3")
    );
}

#[test]
fn build_prompt_blocks_prefixes_instructions_and_metadata_for_fresh_sessions() {
    let options = ProviderRunOptions {
        thread_id: "thread::1".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: Some(vec![ImagePayload {
            name: "sample.png".to_owned(),
            data: "abc".to_owned(),
            media_type: "image/png".to_owned(),
        }]),
        metadata: HashMap::from([(
            "runtime_context".to_owned(),
            json!({
                "channel": "telegram",
                "account_id": "bot1",
                "bot_id": "telegram:bot1",
                "task": {
                    "task_id": "#TASK-2",
                    "status": "in_progress"
                }
            }),
        )]),
    };
    let fresh = build_prompt_blocks(&options, None, true);
    let resumed = build_prompt_blocks(&options, None, false);
    let fresh_text = fresh[0]["text"].as_str().unwrap_or_default();
    assert!(fresh_text.contains("<system_instructions>"));
    assert!(fresh_text.contains("System capabilities:"));
    assert!(fresh_text.contains("<garyx_thread_metadata>"));
    assert!(fresh_text.contains("bot_id: telegram:bot1"));
    assert!(fresh_text.contains("task_id: #TASK-2"));
    assert!(!fresh_text.contains("</garyx_memory_context>"));
    assert!(!fresh_text.contains("<agent_memory"));
    assert!(!fresh_text.contains("status=in_progress"));
    assert!(fresh_text.contains("hello"));
    assert_eq!(resumed[0]["text"], "hello");
    assert_eq!(fresh[1]["type"], "image");
}

#[test]
fn build_prompt_blocks_prepends_memory_for_custom_agents() {
    let options = ProviderRunOptions {
        thread_id: "thread::1".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([("agent_id".to_owned(), json!("reviewer"))]),
    };

    let fresh = build_prompt_blocks(&options, None, true);
    let fresh_text = fresh[0]["text"].as_str().unwrap_or_default();

    assert!(fresh_text.contains("<garyx_memory_context>"));
    assert!(fresh_text.contains("<agent_memory agent_id=\"reviewer\""));
    assert!(fresh_text.contains("hello"));
}

#[test]
fn build_prompt_blocks_skips_agent_memory_for_builtin_gemini() {
    let options = ProviderRunOptions {
        thread_id: "thread::1".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([("agent_id".to_owned(), json!("gemini"))]),
    };

    let fresh = build_prompt_blocks(&options, None, true);
    let fresh_text = fresh[0]["text"].as_str().unwrap_or_default();

    assert!(!fresh_text.contains("</garyx_memory_context>"));
    assert!(!fresh_text.contains("<agent_memory"));
    assert!(fresh_text.contains("hello"));
}

#[test]
fn rewrite_gemini_session_lines_for_fork_replaces_session_ids() {
    let parent = r#"{"sessionId":"parent-session","role":"user","content":"hello"}
{"sessionId":"parent-session","role":"model","content":"world"}
"#;

    let rewritten =
        rewrite_gemini_session_lines_for_fork(parent, "child-session").expect("rewrite");

    assert!(!rewritten.contains("parent-session"));
    assert_eq!(
        rewritten
            .lines()
            .filter(|line| serde_json::from_str::<Value>(line)
                .ok()
                .and_then(|value| value
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(|session_id| session_id == "child-session"))
                .unwrap_or(false))
            .count(),
        2
    );
    assert!(rewritten.contains("\"content\":\"hello\""));
    assert!(rewritten.contains("\"content\":\"world\""));
}

#[test]
fn fork_gemini_session_file_copies_without_mutating_parent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = temp.path().join("session-parent.jsonl");
    let parent = r#"{"sessionId":"parent-session","role":"user","content":"hello"}
{"sessionId":"parent-session","role":"model","content":"world"}
"#;
    fs::write(&source, parent).expect("write source");

    let child_session_id = fork_gemini_session_file(&source).expect("fork session file");

    assert_ne!(child_session_id, "parent-session");
    assert_eq!(fs::read_to_string(&source).expect("read source"), parent);

    let mut children = fs::read_dir(temp.path())
        .expect("read temp")
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path != source).then_some(path)
        })
        .collect::<Vec<_>>();
    children.sort();
    assert_eq!(children.len(), 1);
    let child_contents = fs::read_to_string(&children[0]).expect("read child");
    assert_eq!(
        gemini_session_id_from_jsonl(&child_contents).as_deref(),
        Some(child_session_id.as_str())
    );
    assert!(!child_contents.contains("parent-session"));
}

#[test]
fn resolve_runtime_gemini_env_exports_task_cli_env() {
    let config = GeminiCliConfig::default();
    let metadata = HashMap::from([
        ("agent_id".to_owned(), json!("gemini")),
        (
            "runtime_context".to_owned(),
            json!({
                "thread_id": "thread::gemini-task",
                "task": {
                    "task_id": "#TASK-5",
                    "status": "in_review",
                    "scope": "telegram/gemini_bot"
                }
            }),
        ),
    ]);

    let env = resolve_runtime_gemini_env(&config, &metadata);

    assert_eq!(
        env.get("GARYX_THREAD_ID").map(String::as_str),
        Some("thread::gemini-task")
    );
    assert_eq!(
        env.get("GARYX_ACTOR").map(String::as_str),
        Some("agent:gemini")
    );
    assert_eq!(
        env.get("GARYX_TASK_ID").map(String::as_str),
        Some("#TASK-5")
    );
}

#[test]
fn tool_message_marks_failed_updates_as_errors() {
    let update = json!({
        "toolCallId": "call-1",
        "title": "Read file",
        "status": "failed",
    });
    let message = tool_message(&update, true);
    assert_eq!(message.tool_use_id.as_deref(), Some("call-1"));
    assert_eq!(message.tool_name.as_deref(), Some("Read file"));
    assert_eq!(message.is_error, Some(true));
}

#[test]
fn tool_message_preserves_gemini_google_web_search_output() {
    let update = json!({
        "toolCallId": "search-1",
        "status": "completed",
        "rawInput": {
            "name": "google_web_search",
            "query": "synthetic product release"
        },
        "toolCalls": [{
            "result": [{
                "functionResponse": {
                    "response": {
                        "output": "Result summary.\n\nSources:\n- [Example Source](https://example.test/source)\n- https://docs.example.test/item"
                    }
                }
            }]
        }]
    });

    let message = tool_message(&update, true);

    assert_eq!(message.tool_name.as_deref(), Some("google_web_search"));
    let search = message
        .metadata
        .get("gemini_search")
        .expect("gemini_search metadata");
    assert_eq!(search["provider"], "gemini_cli");
    assert!(
        search["output"]
            .as_str()
            .unwrap_or_default()
            .contains("Result summary")
    );
    let sources = search["sources"].as_array().expect("sources");
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0]["title"], "Example Source");
    assert_eq!(sources[0]["url"], "https://example.test/source");
    assert_eq!(sources[1]["url"], "https://docs.example.test/item");
}

#[test]
fn gemini_search_metadata_ignores_non_search_tools_without_outputs() {
    let update = json!({
        "toolCallId": "read-1",
        "status": "completed",
        "title": "Read file"
    });

    assert!(gemini_search_metadata(&update, Some("Read file")).is_none());
}

#[test]
fn gemini_search_metadata_ignores_named_non_search_tools_with_outputs() {
    let update = json!({
        "toolCallId": "read-1",
        "status": "completed",
        "rawInput": {
            "name": "read_file"
        },
        "toolCalls": [{
            "result": [{
                "functionResponse": {
                    "response": {
                        "output": "Read file output with https://example.test/link"
                    }
                }
            }]
        }]
    });

    assert!(gemini_search_metadata(&update, Some("read_file")).is_none());
}

#[test]
fn gemini_search_metadata_requires_a_search_like_tool_name() {
    let update = json!({
        "toolCallId": "unknown-1",
        "status": "completed",
        "toolCalls": [{
            "result": [{
                "functionResponse": {
                    "response": {
                        "output": "Output with https://example.test/link"
                    }
                }
            }]
        }]
    });

    assert!(gemini_search_metadata(&update, None).is_none());
}

#[test]
fn extract_gemini_thread_title_prefers_update_topic_raw_input() {
    let update = json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "update_topic-1",
        "title": "Update topic to: \"Fallback\"",
        "rawInput": {
            "title": "  Researching   Strings  "
        }
    });

    assert_eq!(
        extract_gemini_thread_title(&update).as_deref(),
        Some("Researching Strings")
    );
}

#[test]
fn extract_gemini_thread_title_parses_update_topic_display_title() {
    let update = json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "update_topic-1",
        "title": "Update topic to: \"Researching Strings\""
    });

    assert_eq!(
        extract_gemini_thread_title(&update).as_deref(),
        Some("Researching Strings")
    );
}

#[test]
fn approval_mode_normalizes_cli_spellings() {
    let config = GeminiCliConfig::default();
    assert_eq!(approval_mode(&config, &HashMap::new()), "yolo");

    let metadata = HashMap::from([("approval_mode".to_owned(), json!("auto_edit"))]);
    assert_eq!(approval_mode(&config, &metadata), "autoEdit");

    let metadata = HashMap::from([("approval_mode".to_owned(), json!("plan"))]);
    assert_eq!(approval_mode(&config, &metadata), "plan");
}

#[test]
fn resolve_session_id_uses_response_value_or_requested_fallback() {
    let response_value = json!({
        "result": {
            "sessionId": "fresh-session"
        }
    });
    assert_eq!(
        resolve_session_id_from_response(&response_value, Some("persisted-session")).unwrap(),
        "fresh-session"
    );

    let missing_value = json!({
        "result": {}
    });
    assert_eq!(
        resolve_session_id_from_response(&missing_value, Some("persisted-session")).unwrap(),
        "persisted-session"
    );

    let err = resolve_session_id_from_response(&missing_value, None).expect_err("expected failure");
    assert!(
        err.to_string().contains("missing sessionId"),
        "unexpected error: {err}"
    );
}

#[test]
fn extract_prompt_result_actual_model_prefers_authoritative_quota_usage() {
    let message = json!({
        "result": {
            "_meta": {
                "quota": {
                    "model_usage": [
                        { "model": "gemini-3-flash-preview" }
                    ]
                }
            }
        }
    });

    assert_eq!(
        extract_prompt_result_actual_model(&message).as_deref(),
        Some("gemini-3-flash-preview")
    );
}

#[test]
fn extract_prompt_result_usage_supports_quota_token_count_shape() {
    let message = json!({
        "result": {
            "_meta": {
                "quota": {
                    "token_count": {
                        "input_tokens": 123,
                        "output_tokens": 45
                    }
                }
            }
        }
    });

    assert_eq!(extract_prompt_result_usage(&message), (123, 45));
}

#[test]
fn strip_gemini_thought_output_keeps_only_visible_tail_after_markers() {
    let raw = concat!(
        "**Thinking one**\n[Thought: true]",
        "**Thinking two**\n[Thought: true]",
        "好的，现在开始执行。"
    );
    assert_eq!(strip_gemini_thought_output(raw), "好的，现在开始执行。");
}

#[test]
fn gemini_provider_uses_current_acp_flag() {
    assert_eq!(GEMINI_ACP_ARG, "--acp");
}

#[tokio::test]
async fn run_streaming_invokes_gemini_with_current_acp_flag() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace_dir = temp.path().join("workspace");
    fs::create_dir_all(&workspace_dir).expect("create workspace");
    let script_path = temp.path().join("fake-gemini-arg-check.py");
    let script = r#"#!/usr/bin/env python3
import json
import sys

if "--version" in sys.argv:
    print("0.0-test")
    sys.exit(0)

if "--acp" not in sys.argv or "--skip-trust" in sys.argv or "--experimental-acp" in sys.argv:
    print("unexpected args: " + " ".join(sys.argv[1:]), file=sys.stderr)
    sys.exit(2)

for line in sys.stdin:
    req = json.loads(line)
    rid = req["id"]
    method = req["method"]
    params = req.get("params", {})

    if method == "initialize":
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": 1}}), flush=True)
    elif method == "session/new":
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {"sessionId": "arg-session"}}), flush=True)
    elif method == "session/set_mode":
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {}}), flush=True)
    elif method == "session/set_model":
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {}}), flush=True)
    elif method == "session/prompt":
        print(json.dumps({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": params.get("sessionId"),
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"text": "OK"}
                }
            }
        }), flush=True)
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {}}), flush=True)
        break
    else:
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "error": {"message": "unsupported"}}), flush=True)
        break
"#;
    fs::write(&script_path, script).expect("write script");
    let mut permissions = fs::metadata(&script_path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("chmod script");

    let mut provider = GeminiCliProvider::new(GeminiCliConfig {
        gemini_bin: script_path.to_string_lossy().to_string(),
        workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
        timeout_seconds: 5.0,
        model: String::new(),
        ..Default::default()
    });
    provider.ready = true;

    let events = StdArc::new(StdMutex::new(Vec::new()));
    let events_for_callback = StdArc::clone(&events);
    let callback: Box<dyn Fn(StreamEvent) + Send + Sync> = Box::new(move |event| {
        events_for_callback.lock().unwrap().push(event);
    });
    let result = provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id: "thread::gemini::arg-check".to_owned(),
                message: "hello".to_owned(),
                workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
                images: None,
                metadata: HashMap::new(),
            },
            callback,
        )
        .await
        .expect("run should succeed");
    assert!(result.success, "run failed: {:?}", result.error);
    assert_eq!(result.response, "OK");
    let events = events.lock().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::Done))
            .count(),
        1,
    );
}

#[tokio::test]
async fn run_streaming_startup_close_reports_stderr() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace_dir = temp.path().join("workspace");
    fs::create_dir_all(&workspace_dir).expect("create workspace");
    let script_path = temp.path().join("fake-gemini-startup-fail.py");
    let script = r#"#!/usr/bin/env python3
import sys

if "--version" in sys.argv:
    print("0.0-test")
    sys.exit(0)

print("Unknown argument: acp", file=sys.stderr)
sys.exit(1)
"#;
    fs::write(&script_path, script).expect("write script");
    let mut permissions = fs::metadata(&script_path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("chmod script");

    let mut provider = GeminiCliProvider::new(GeminiCliConfig {
        gemini_bin: script_path.to_string_lossy().to_string(),
        workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
        timeout_seconds: 5.0,
        model: String::new(),
        ..Default::default()
    });
    provider.ready = true;

    let callback: Box<dyn Fn(StreamEvent) + Send + Sync> = Box::new(|_| {});
    let error = provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id: "thread::gemini::startup-fail".to_owned(),
                message: "hello".to_owned(),
                workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
                images: None,
                metadata: HashMap::new(),
            },
            callback,
        )
        .await
        .expect_err("startup failure should be surfaced");
    let message = error.to_string();
    assert!(message.contains("gemini ACP closed before responding"));
    assert!(message.contains("Unknown argument: acp"));
}

#[tokio::test]
async fn run_streaming_reuses_loaded_session_when_load_response_omits_session_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace_dir = temp.path().join("workspace");
    fs::create_dir_all(&workspace_dir).expect("create workspace");
    let counter_path = temp.path().join("counter.txt");
    let script_path = temp.path().join("fake-gemini-acp.py");
    let script = r#"#!/usr/bin/env python3
import json
import os
import pathlib
import sys

if "--version" in sys.argv:
    print("0.0-test")
    sys.exit(0)

counter_path = pathlib.Path(os.environ["COUNTER_FILE"])
if counter_path.exists():
    invocation = int(counter_path.read_text()) + 1
else:
    invocation = 1
counter_path.write_text(str(invocation))

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    rid = req["id"]
    method = req["method"]
    params = req.get("params", {})

    if method == "initialize":
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": 1}}), flush=True)
    elif method == "session/new":
        assert invocation == 1, f"unexpected session/new on invocation {invocation}"
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {"sessionId": "persisted-session"}}), flush=True)
    elif method == "session/load":
        assert invocation == 2, f"unexpected session/load on invocation {invocation}"
        assert params.get("sessionId") == "persisted-session"
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {}}), flush=True)
    elif method == "session/set_mode":
        assert params.get("sessionId") == "persisted-session"
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {}}), flush=True)
    elif method == "session/set_model":
        assert params.get("sessionId") == "persisted-session"
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {}}), flush=True)
    elif method == "session/prompt":
        assert params.get("sessionId") == "persisted-session"
        text = "FIRST TURN" if invocation == 1 else "SECOND TURN"
        print(json.dumps({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "persisted-session",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"text": text}
                }
            }
        }), flush=True)
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": rid,
            "result": {
                "usage": {
                    "inputTokens": 1,
                    "outputTokens": 1
                }
            }
        }), flush=True)
        break
    else:
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": rid,
            "error": {"message": f"unsupported method: {method}"}
        }), flush=True)
        break
"#;
    fs::write(&script_path, script).expect("write script");
    let mut permissions = fs::metadata(&script_path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("chmod script");

    let mut provider = GeminiCliProvider::new(GeminiCliConfig {
        gemini_bin: script_path.to_string_lossy().to_string(),
        workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
        timeout_seconds: 5.0,
        model: "gemini-3-flash-preview".to_owned(),
        env: HashMap::from([(
            "COUNTER_FILE".to_owned(),
            counter_path.to_string_lossy().to_string(),
        )]),
        ..Default::default()
    });
    provider.ready = true;

    let callback: Box<dyn Fn(StreamEvent) + Send + Sync> = Box::new(|_| {});
    let first = provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id: "thread::gemini::resume".to_owned(),
                message: "first".to_owned(),
                workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
                images: None,
                metadata: HashMap::new(),
            },
            callback,
        )
        .await
        .expect("first run should succeed");
    assert!(first.success, "first run failed: {:?}", first.error);
    assert_eq!(first.response, "FIRST TURN");
    assert_eq!(first.sdk_session_id.as_deref(), Some("persisted-session"));

    let callback: Box<dyn Fn(StreamEvent) + Send + Sync> = Box::new(|_| {});
    let second = provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id: "thread::gemini::resume".to_owned(),
                message: "second".to_owned(),
                workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
                images: None,
                metadata: HashMap::new(),
            },
            callback,
        )
        .await
        .expect("second run should succeed");
    assert!(second.success, "second run failed: {:?}", second.error);
    assert_eq!(second.response, "SECOND TURN");
    assert_eq!(second.sdk_session_id.as_deref(), Some("persisted-session"));
    assert_eq!(
        provider
            .session_map
            .lock()
            .await
            .get("thread::gemini::resume")
            .map(String::as_str),
        Some("persisted-session")
    );
}

#[tokio::test]
async fn run_streaming_extends_idle_timeout_while_tool_work_is_in_progress() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace_dir = temp.path().join("workspace");
    fs::create_dir_all(&workspace_dir).expect("create workspace");
    let script_path = temp.path().join("fake-gemini-tool-wait.py");
    let script = r#"#!/usr/bin/env python3
import json
import sys
import time

if "--version" in sys.argv:
    print("0.0-test")
    sys.exit(0)

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    rid = req["id"]
    method = req["method"]
    params = req.get("params", {})

    if method == "initialize":
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": 1}}), flush=True)
    elif method == "session/new":
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {"sessionId": "tool-session"}}), flush=True)
    elif method == "session/set_mode":
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {}}), flush=True)
    elif method == "session/set_model":
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {}}), flush=True)
    elif method == "session/prompt":
        print(json.dumps({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "tool-session",
                "update": {
                    "sessionUpdate": "tool_call",
                    "toolCallId": "generalist-1",
                    "kind": "think",
                    "status": "in_progress",
                    "title": "Delegating to agent 'generalist'"
                }
            }
        }), flush=True)
        time.sleep(5.5)
        print(json.dumps({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "tool-session",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"text": "DONE"}
                }
            }
        }), flush=True)
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": rid,
            "result": {
                "_meta": {
                    "quota": {
                        "token_count": {"input_tokens": 1, "output_tokens": 1},
                        "model_usage": [{"model": "gemini-3.1-pro-preview"}]
                    }
                }
            }
        }), flush=True)
        break
    else:
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "error": {"message": f"unsupported method: {method}"}}), flush=True)
        break
"#;
    fs::write(&script_path, script).expect("write script");
    let mut permissions = fs::metadata(&script_path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("chmod script");

    let mut provider = GeminiCliProvider::new(GeminiCliConfig {
        gemini_bin: script_path.to_string_lossy().to_string(),
        workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
        timeout_seconds: 5.0,
        model: "gemini-3.1-pro-preview".to_owned(),
        ..Default::default()
    });
    provider.ready = true;

    let callback: Box<dyn Fn(StreamEvent) + Send + Sync> = Box::new(|_| {});
    let result = provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id: "thread::gemini::tool-timeout".to_owned(),
                message: "delegate".to_owned(),
                workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
                images: None,
                metadata: HashMap::new(),
            },
            callback,
        )
        .await
        .expect("run should succeed after extending idle timeout");

    assert!(result.success, "run failed: {:?}", result.error);
    assert_eq!(result.response, "DONE");
    assert_eq!(
        result.actual_model.as_deref(),
        Some("gemini-3.1-pro-preview")
    );
    assert!(
        result
            .session_messages
            .iter()
            .any(|message| message.role == ProviderMessageRole::ToolUse
                && message.tool_name.as_deref() == Some("Delegating to agent 'generalist'")),
        "expected delegation tool trace in session messages"
    );
}

#[tokio::test]
async fn run_streaming_strips_thought_marked_output_and_emits_only_visible_text() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace_dir = temp.path().join("workspace");
    fs::create_dir_all(&workspace_dir).expect("create workspace");
    let script_path = temp.path().join("fake-gemini-thought.py");
    let script = r#"#!/usr/bin/env python3
import json
import sys

if "--version" in sys.argv:
    print("0.0-test")
    sys.exit(0)

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    rid = req["id"]
    method = req["method"]
    params = req.get("params", {})

    if method == "initialize":
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": 1}}), flush=True)
    elif method == "session/new":
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {"sessionId": "thought-session"}}), flush=True)
    elif method == "session/set_mode":
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {}}), flush=True)
    elif method == "session/set_model":
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "result": {}}), flush=True)
    elif method == "session/prompt":
        print(json.dumps({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "thought-session",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"text": "**Thinking one**\n[Thought: true]"}
                }
            }
        }), flush=True)
        print(json.dumps({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "thought-session",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"text": "**Thinking two**\n[Thought: true]好的，开始。"}
                }
            }
        }), flush=True)
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": rid,
            "result": {
                "_meta": {
                    "quota": {
                        "token_count": {"input_tokens": 1, "output_tokens": 2},
                        "model_usage": [{"model": "gemini-3-flash-preview"}]
                    }
                }
            }
        }), flush=True)
        break
    else:
        print(json.dumps({"jsonrpc": "2.0", "id": rid, "error": {"message": f"unsupported method: {method}"}}), flush=True)
        break
"#;
    fs::write(&script_path, script).expect("write script");
    let mut permissions = fs::metadata(&script_path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("chmod script");

    let mut provider = GeminiCliProvider::new(GeminiCliConfig {
        gemini_bin: script_path.to_string_lossy().to_string(),
        workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
        timeout_seconds: 5.0,
        model: "gemini-3-flash-preview".to_owned(),
        ..Default::default()
    });
    provider.ready = true;

    let emitted = Arc::new(std::sync::Mutex::new(String::new()));
    let emitted_clone = emitted.clone();
    let callback: Box<dyn Fn(StreamEvent) + Send + Sync> = Box::new(move |event| {
        if let StreamEvent::Delta { text } = event {
            emitted_clone.lock().unwrap().push_str(&text);
        }
    });
    let result = provider
        .run_streaming(
            &ProviderRunOptions {
                thread_id: "thread::gemini::thought".to_owned(),
                message: "sanitize".to_owned(),
                workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
                images: None,
                metadata: HashMap::new(),
            },
            callback,
        )
        .await
        .expect("run should succeed");

    assert_eq!(result.response, "好的，开始。");
    assert_eq!(
        result.actual_model.as_deref(),
        Some("gemini-3-flash-preview")
    );
    assert_eq!(emitted.lock().unwrap().as_str(), "好的，开始。");
    let assistant = result
        .session_messages
        .iter()
        .find(|message| message.role == ProviderMessageRole::Assistant)
        .expect("assistant message");
    assert_eq!(assistant.text.as_deref(), Some("好的，开始。"));
}

#[tokio::test]
async fn run_streaming_keeps_started_run_model_when_defaults_reload_mid_run() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace_dir = temp.path().join("workspace");
    fs::create_dir_all(&workspace_dir).expect("create workspace");
    let started_path = temp.path().join("fake-started");
    let release_path = temp.path().join("fake-release");
    let model_path = temp.path().join("fake-set-model");
    let script_path = temp.path().join("fake-gemini-mid-run-reload.py");
    let script = format!(
        r#"#!/usr/bin/env python3
import json
import os
import sys
import time

if "--version" in sys.argv:
    print("0.0-test")
    sys.exit(0)

STARTED = {started:?}
RELEASE = {release:?}
MODEL = {model:?}

for line in sys.stdin:
    req = json.loads(line)
    rid = req["id"]
    method = req["method"]
    params = req.get("params", {{}})

    if method == "initialize":
        # Signal the test that this run is live, then hold the ACP handshake
        # until the test has hot-reloaded the provider defaults.
        open(STARTED, "w").write("1")
        for _ in range(500):
            if os.path.exists(RELEASE):
                break
            time.sleep(0.01)
        print(json.dumps({{"jsonrpc": "2.0", "id": rid, "result": {{"protocolVersion": 1}}}}), flush=True)
    elif method == "session/new":
        print(json.dumps({{"jsonrpc": "2.0", "id": rid, "result": {{"sessionId": "mid-run-session"}}}}), flush=True)
    elif method == "session/set_mode":
        print(json.dumps({{"jsonrpc": "2.0", "id": rid, "result": {{}}}}), flush=True)
    elif method == "session/set_model":
        open(MODEL, "w").write(params.get("modelId", ""))
        print(json.dumps({{"jsonrpc": "2.0", "id": rid, "result": {{}}}}), flush=True)
    elif method == "session/prompt":
        print(json.dumps({{
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {{
                "sessionId": params.get("sessionId"),
                "update": {{
                    "sessionUpdate": "agent_message_chunk",
                    "content": {{"text": "OK"}}
                }}
            }}
        }}), flush=True)
        print(json.dumps({{"jsonrpc": "2.0", "id": rid, "result": {{}}}}), flush=True)
        break
    else:
        print(json.dumps({{"jsonrpc": "2.0", "id": rid, "error": {{"message": "unsupported"}}}}), flush=True)
        break
"#,
        started = started_path.to_string_lossy(),
        release = release_path.to_string_lossy(),
        model = model_path.to_string_lossy(),
    );
    fs::write(&script_path, script).expect("write script");
    let mut permissions = fs::metadata(&script_path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("chmod script");

    let mut provider = GeminiCliProvider::new(GeminiCliConfig {
        gemini_bin: script_path.to_string_lossy().to_string(),
        workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
        timeout_seconds: 15.0,
        model: "gemini-old".to_owned(),
        ..Default::default()
    });
    provider.ready = true;

    let callback: Box<dyn Fn(StreamEvent) + Send + Sync> = Box::new(|_| {});
    let run_options = ProviderRunOptions {
        thread_id: "thread::gemini::mid-run-reload".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
        images: None,
        metadata: HashMap::new(),
    };
    let run_future = provider.run_streaming(&run_options, callback);

    let orchestrate = async {
        // Wait until the fake CLI is live (the run is spawned and registered).
        for _ in 0..500 {
            if started_path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(started_path.exists(), "fake gemini CLI never started");
        // Hot-reload the provider defaults while the run is in flight.
        provider.update_model_defaults(&ProviderModelDefaults {
            model: "gemini-new".to_owned(),
            default_model: "gemini-new".to_owned(),
            model_reasoning_effort: String::new(),
            model_service_tier: String::new(),
        });
        fs::write(&release_path, "1").expect("write release signal");
    };

    let (result, ()) = tokio::join!(run_future, orchestrate);
    let result = result.expect("run should succeed");
    assert!(result.success, "run failed: {:?}", result.error);

    let sent_model = fs::read_to_string(&model_path).expect("fake CLI recorded set_model");
    assert_eq!(
        sent_model, "gemini-old",
        "an already-started run must keep the model captured at run start, \
         not pick up defaults reloaded mid-run"
    );
}

#[test]
fn resolve_runtime_gemini_env_overlays_agent_config_env() {
    // config.env is the agent's configured env; it must appear in the map that
    // is fed verbatim to `command.envs(...)` on the very next line of the spawn.
    let mut config = GeminiCliConfig::default();
    config
        .env
        .insert("TEST_AGENT_ENV_KEY".to_owned(), "test-value".to_owned());
    // Agent env must NOT be able to override GARYX identity vars (task_cli_env
    // overlays config.env, so identity wins).
    config
        .env
        .insert("GARYX_THREAD_ID".to_owned(), "agent-should-not-win".to_owned());
    let metadata = HashMap::from([
        ("agent_id".to_owned(), json!("gemini")),
        (
            "runtime_context".to_owned(),
            json!({ "thread_id": "thread::gemini-task" }),
        ),
    ]);

    let env = resolve_runtime_gemini_env(&config, &metadata);

    assert_eq!(
        env.get("TEST_AGENT_ENV_KEY").map(String::as_str),
        Some("test-value")
    );
    assert_eq!(
        env.get("GARYX_THREAD_ID").map(String::as_str),
        Some("thread::gemini-task"),
        "task identity env must overlay (win over) agent config env"
    );
}
