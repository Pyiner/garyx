use super::*;
use garyx_models::provider::{ProviderMessage, ProviderRunResult};
fn run_result(response: &str, session_messages: Vec<ProviderMessage>) -> ProviderRunResult {
    ProviderRunResult {
        run_id: "run-1".to_owned(),
        thread_id: "thread::1".to_owned(),
        response: response.to_owned(),
        session_messages,
        sdk_session_id: None,
        actual_model: None,
        success: true,
        error: None,
        input_tokens: 0,
        output_tokens: 0,
        cost: 0.0,
        duration_ms: 0,
    }
}

#[test]
fn auto_disables_loop_for_no_work_reply() {
    let result = run_result("当前没有剩余代码任务。", vec![]);
    let metadata = HashMap::new();
    assert!(should_auto_disable_loop(&metadata, &result));
}

#[test]
fn auto_disables_loop_for_explicit_text_only_reply() {
    let result = run_result("loop 已停止。", vec![]);
    let metadata = HashMap::new();
    assert!(should_auto_disable_loop(&metadata, &result));
}

#[test]
fn keeps_loop_for_empty_text_only_reply() {
    let result = run_result("   ", vec![]);
    let metadata = HashMap::new();
    assert!(!should_auto_disable_loop(&metadata, &result));
}

#[test]
fn keeps_loop_when_tools_were_used() {
    let result = run_result(
        "当前没有剩余代码任务。",
        vec![ProviderMessage::tool_use(
            Value::Object(Default::default()),
            Some("tool-1".to_owned()),
            Some("mcp:garyx:status".to_owned()),
        )],
    );
    let metadata = HashMap::new();
    assert!(!should_auto_disable_loop(&metadata, &result));
}

#[test]
fn auto_disables_loop_for_first_run_without_tools() {
    let result = run_result("任务已完成。", vec![]);
    assert!(should_auto_disable_loop(&HashMap::new(), &result));
}

#[test]
fn test_resolve_sdk_session_id_for_persistence_prefers_non_empty_result() {
    let mut metadata = HashMap::new();
    metadata.insert("sdk_session_id".to_owned(), json!("persisted-session"));

    let resolved = resolve_sdk_session_id_for_persistence(&metadata, Some("new-session"));

    assert_eq!(resolved.as_deref(), Some("new-session"));
}

#[test]
fn test_resolve_sdk_session_id_for_persistence_falls_back_to_metadata() {
    let mut metadata = HashMap::new();
    metadata.insert("sdk_session_id".to_owned(), json!("persisted-session"));

    let resolved = resolve_sdk_session_id_for_persistence(&metadata, Some("   "));

    assert_eq!(resolved.as_deref(), Some("persisted-session"));
}

#[test]
fn test_resolve_sdk_session_id_for_persistence_ignores_empty_values() {
    let mut metadata = HashMap::new();
    metadata.insert("sdk_session_id".to_owned(), json!("   "));

    let resolved = resolve_sdk_session_id_for_persistence(&metadata, None);

    assert!(resolved.is_none());
}

#[test]
fn test_resolve_persisted_sdk_session_id_for_provider_prefers_provider_scoped_value() {
    let session_data = json!({
        "provider_key": "claude",
        "sdk_session_id": "legacy-session",
        "provider_sdk_session_ids": {
            "claude": "claude-session",
            "codex": "codex-thread"
        }
    });

    let resolved = resolve_persisted_sdk_session_id_for_provider(&session_data, "claude", None);

    assert_eq!(resolved.as_deref(), Some("claude-session"));
}

#[test]
fn test_resolve_persisted_sdk_session_id_for_provider_falls_back_to_matching_legacy_value() {
    let session_data = json!({
        "provider_key": "claude",
        "sdk_session_id": "legacy-session",
    });

    let resolved = resolve_persisted_sdk_session_id_for_provider(&session_data, "claude", None);

    assert_eq!(resolved.as_deref(), Some("legacy-session"));
}

#[test]
fn test_resolve_persisted_sdk_session_id_for_provider_ignores_other_provider_legacy_value() {
    let session_data = json!({
        "provider_key": "codex",
        "sdk_session_id": "codex-thread",
    });

    let resolved = resolve_persisted_sdk_session_id_for_provider(&session_data, "claude", None);

    assert!(resolved.is_none());
}

#[test]
fn test_build_group_transcript_snapshot_uses_user_label_for_human_turns() {
    let thread_data = json!({
        "messages": [
            {
                "role": "user",
                "text": "@[Coder](coder) please help",
                "timestamp": "t0",
                "metadata": {
                    "agent_id": "team::demo",
                    "from_id": "alice"
                }
            },
            {
                "role": "assistant",
                "text": "On it.",
                "timestamp": "t1",
                "metadata": {
                    "agent_id": "coder",
                    "agent_display_name": "Coder"
                }
            },
            {
                "role": "user",
                "text": "@[Reviewer](reviewer) take a look",
                "timestamp": "t2",
                "metadata": {
                    "internal_dispatch": true,
                    "agent_id": "planner",
                    "agent_display_name": "Planner"
                }
            }
        ]
    });

    let snapshot = build_group_transcript_snapshot(&thread_data);

    assert_eq!(
        snapshot,
        json!([
            {"agent_id": "user", "text": "@[Coder](coder) please help", "at": "t0"},
            {"agent_id": "coder", "text": "On it.", "at": "t1"},
            {"agent_id": "planner", "text": "@[Reviewer](reviewer) take a look", "at": "t2"}
        ])
    );
}
