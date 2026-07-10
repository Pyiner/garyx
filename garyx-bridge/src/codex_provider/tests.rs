use super::*;
use serde_json::json;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicUsize, Ordering};

// -- Helper function tests --

#[test]
fn test_matches_turn_exact() {
    let params = json!({"threadId": "t1", "turnId": "u1"});
    assert!(matches_turn(&params, "t1", "u1"));
}

#[test]
fn codex_error_usage_limit_detects_string_and_object_forms() {
    assert!(codex_error_is_usage_limit(Some(&json!(
        "usageLimitExceeded"
    ))));
    assert!(codex_error_is_usage_limit(Some(
        &json!({ "usageLimitExceeded": {} })
    )));
    assert!(!codex_error_is_usage_limit(Some(&json!(
        "serverOverloaded"
    ))));
    assert!(!codex_error_is_usage_limit(None));
}

#[test]
fn build_codex_rate_limit_uses_snapshot_reset_for_primary_window() {
    let snapshot = json!({
        "planType": "pro",
        "rateLimitReachedType": "rate_limit_reached",
        "primary": { "usedPercent": 100, "resetsAt": 1_893_477_600_i64, "windowDurationMins": 300 },
        "secondary": { "usedPercent": 42, "resetsAt": 1_893_900_000_i64, "windowDurationMins": 10080 }
    });
    let rate_limit = build_codex_rate_limit(
        "codex_app_server",
        true,
        Some(&snapshot),
        Some("You've hit your usage limit."),
    )
    .expect("rate limit built");
    assert_eq!(rate_limit.provider, "codex_app_server");
    assert_eq!(rate_limit.window.as_deref(), Some("primary"));
    assert_eq!(rate_limit.used_percent, Some(100));
    assert_eq!(
        rate_limit.reset_at.as_deref(),
        Some("2030-01-01T06:00:00+00:00")
    );
    assert_eq!(
        rate_limit.reached_type.as_deref(),
        Some("rate_limit_reached")
    );
}

#[test]
fn build_codex_rate_limit_picks_weekly_window_when_it_is_the_exhausted_one() {
    let snapshot = json!({
        "primary": { "usedPercent": 80, "resetsAt": 1_893_477_600_i64 },
        "secondary": { "usedPercent": 100, "resetsAt": 1_893_900_000_i64 }
    });
    let rate_limit =
        build_codex_rate_limit("codex_app_server", true, Some(&snapshot), None).expect("built");
    assert_eq!(rate_limit.window.as_deref(), Some("secondary"));
    assert_eq!(
        rate_limit.reset_at.as_deref(),
        Some("2030-01-06T03:20:00+00:00")
    );
}

#[test]
fn build_codex_rate_limit_returns_none_without_quota_signal() {
    let snapshot = json!({
        "primary": { "usedPercent": 30, "resetsAt": 1_893_477_600_i64 },
        "secondary": { "usedPercent": 10 }
    });
    assert!(build_codex_rate_limit("codex_app_server", false, Some(&snapshot), None).is_none());
    assert!(build_codex_rate_limit("codex_app_server", false, None, None).is_none());
}

#[test]
fn build_codex_rate_limit_ignores_bare_saturation_without_explicit_signal() {
    // A window at 100% but no `usageLimitExceeded` error and no
    // `rateLimitReachedType` must NOT be classified as a quota failure — the run
    // failed for an unrelated reason while usage happened to be saturated.
    let snapshot = json!({
        "primary": { "usedPercent": 100, "resetsAt": 1_893_477_600_i64 },
        "secondary": { "usedPercent": 40 }
    });
    assert!(build_codex_rate_limit("codex_app_server", false, Some(&snapshot), None).is_none());

    // The same saturated snapshot WITH the explicit error is a real quota hit.
    let rate_limit =
        build_codex_rate_limit("codex_app_server", true, Some(&snapshot), None).expect("built");
    assert_eq!(rate_limit.window.as_deref(), Some("primary"));

    // A `rateLimitReachedType` alone (no per-turn error) also qualifies.
    let reached = json!({
        "rateLimitReachedType": "rate_limit_reached",
        "primary": { "usedPercent": 100, "resetsAt": 1_893_477_600_i64 }
    });
    assert!(build_codex_rate_limit("codex_app_server", false, Some(&reached), None).is_some());
}

fn local_now(hour: u32, minute: u32) -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone;
    chrono::Local
        .with_ymd_and_hms(2026, 7, 10, hour, minute, 0)
        .single()
        .expect("mid-July local time resolves")
}

fn parsed_offset_minutes(reset_at: &str, now: chrono::DateTime<chrono::Local>) -> i64 {
    let reset = chrono::DateTime::parse_from_rfc3339(reset_at).expect("rfc3339 reset");
    (reset.with_timezone(&chrono::Utc) - now.with_timezone(&chrono::Utc)).num_minutes()
}

#[test]
fn reset_from_message_parses_wall_clock_time_later_today() {
    let now = local_now(21, 0);
    let reset = reset_at_from_usage_message(
        "You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at 9:42 PM.",
        now,
    )
    .expect("reset parsed");
    assert_eq!(parsed_offset_minutes(&reset, now), 42);
}

#[test]
fn reset_from_message_rolls_past_time_to_tomorrow() {
    let now = local_now(22, 0);
    let reset = reset_at_from_usage_message("or try again at 9:42 PM.", now).expect("parsed");
    assert_eq!(parsed_offset_minutes(&reset, now), 23 * 60 + 42);
}

#[test]
fn reset_from_message_keeps_just_recovered_time_today() {
    // Within the five-minute slack the window just recovered; the reset must
    // stay today (slightly in the past), not jump a full day out.
    let now = local_now(21, 44);
    let reset = reset_at_from_usage_message("try again at 9:42 PM", now).expect("parsed");
    assert_eq!(parsed_offset_minutes(&reset, now), -2);
}

#[test]
fn reset_from_message_maps_twelve_am_pm_correctly() {
    let now = local_now(1, 0);
    let midnight = reset_at_from_usage_message("try again at 12 AM", now).expect("parsed");
    assert_eq!(parsed_offset_minutes(&midnight, now), 23 * 60);

    let noon = reset_at_from_usage_message("try again at 12:30 PM", now).expect("parsed");
    assert_eq!(parsed_offset_minutes(&noon, now), 11 * 60 + 30);
}

#[test]
fn reset_from_message_parses_relative_durations() {
    let now = local_now(9, 0);
    let combined = reset_at_from_usage_message("Please try again in 2 hours 13 minutes.", now)
        .expect("parsed");
    assert_eq!(parsed_offset_minutes(&combined, now), 133);

    let minutes_only = reset_at_from_usage_message("try again in 45 minutes", now).expect("parsed");
    assert_eq!(parsed_offset_minutes(&minutes_only, now), 45);
}

#[test]
fn reset_from_message_returns_none_without_reset_hint() {
    let now = local_now(9, 0);
    assert!(
        reset_at_from_usage_message(
            "You've hit your usage limit. Visit the settings page to purchase more credits.",
            now,
        )
        .is_none()
    );
    assert!(reset_at_from_usage_message("try again at 25:99 PM", now).is_none());
    assert!(reset_at_from_usage_message("try again shortly", now).is_none());
}

#[test]
fn reset_from_message_rejects_word_boundary_false_positives() {
    let now = local_now(9, 0);
    // "PMaybe" is not an AM/PM marker.
    assert!(reset_at_from_usage_message("try again at 9 PMaybe later", now).is_none());
}

#[test]
fn reset_from_message_survives_absurd_durations_without_panicking() {
    let now = local_now(9, 0);
    // Duration overflow from a malformed upstream message must not panic.
    assert!(reset_at_from_usage_message("try again in 9223372036854775807 days", now).is_none());
    // Amounts past the plausibility cap are rejected too.
    assert!(reset_at_from_usage_message("try again in 4000 days", now).is_none());
}

#[test]
fn reset_from_message_ambiguous_dst_time_takes_earliest_instant() {
    use chrono::TimeZone;
    // America/New_York, 2026-11-01: clocks fall back at 02:00 EDT → 01:00 EST,
    // so 01:30 occurs twice (05:30Z EDT and 06:30Z EST). The parse must pick
    // the earliest UTC instant regardless of chrono's Ambiguous pair order.
    let zone: chrono_tz::Tz = "America/New_York".parse().expect("zone");
    let now = zone
        .with_ymd_and_hms(2026, 11, 1, 0, 30, 0)
        .single()
        .expect("unambiguous midnight-thirty");
    let reset = reset_at_from_usage_message_in("try again at 1:30 AM", now).expect("parsed");
    assert_eq!(reset, "2026-11-01T05:30:00+00:00");
}

#[test]
fn reset_from_message_dst_gap_time_slides_forward() {
    use chrono::TimeZone;
    // America/New_York, 2026-03-08: 02:00–03:00 does not exist (spring
    // forward). A 2:30 AM hint resolves an hour later instead of failing.
    let zone: chrono_tz::Tz = "America/New_York".parse().expect("zone");
    let now = zone
        .with_ymd_and_hms(2026, 3, 8, 0, 30, 0)
        .single()
        .expect("unambiguous half past midnight");
    let reset = reset_at_from_usage_message_in("try again at 2:30 AM", now).expect("parsed");
    assert_eq!(reset, "2026-03-08T07:30:00+00:00");
}

#[test]
fn build_codex_rate_limit_falls_back_to_message_reset_without_snapshot() {
    // The real-world Codex shape: usage-limit error with no structured
    // snapshot, reset time only in the message. `reset_at` presence is what
    // activates the countdown banner and quota auto-resend downstream.
    let rate_limit = build_codex_rate_limit(
        "codex_app_server",
        true,
        None,
        Some("You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at 9:42 PM."),
    )
    .expect("rate limit built");
    assert!(rate_limit.reset_at.is_some());
    assert!(rate_limit.window.is_none());
    assert!(rate_limit.message.is_some());
}

#[test]
fn extract_rate_limit_snapshot_accepts_wrapped_and_flattened_shapes() {
    let wrapped = json!({ "rateLimits": { "primary": { "usedPercent": 50 } } });
    assert_eq!(
        extract_rate_limit_snapshot(&wrapped),
        Some(json!({ "primary": { "usedPercent": 50 } }))
    );

    let flattened = json!({ "primary": { "usedPercent": 50 }, "secondary": { "usedPercent": 10 } });
    assert_eq!(
        extract_rate_limit_snapshot(&flattened),
        Some(flattened.clone())
    );

    assert_eq!(
        extract_rate_limit_snapshot(&json!({ "unrelated": 1 })),
        None
    );
}

#[test]
fn test_matches_turn_wrong_thread() {
    let params = json!({"threadId": "t2", "turnId": "u1"});
    assert!(!matches_turn(&params, "t1", "u1"));
}

#[test]
fn test_matches_turn_wrong_turn() {
    let params = json!({"threadId": "t1", "turnId": "u2"});
    assert!(!matches_turn(&params, "t1", "u1"));
}

#[test]
fn test_matches_turn_no_ids_matches() {
    let params = json!({"data": 42});
    assert!(matches_turn(&params, "t1", "u1"));
}

#[test]
fn test_codex_run_result_timed_out_detects_provider_timeout() {
    let result = Ok(ProviderRunResult {
        run_id: "run-1".to_owned(),
        thread_id: "thread-1".to_owned(),
        response: String::new(),
        session_messages: Vec::new(),
        sdk_session_id: Some("codex-thread-1".to_owned()),
        actual_model: None,
        thread_title: None,
        success: false,
        error: Some("timeout".to_owned()),
        input_tokens: 0,
        output_tokens: 0,
        cost: 0.0,
        duration_ms: 300_000,
    });

    assert!(codex_run_result_timed_out(&result));
    assert!(codex_run_result_timed_out(&Err(BridgeError::Timeout)));
}

#[test]
fn test_codex_timeout_auto_continue_options_replaces_message_only() {
    let options = ProviderRunOptions {
        thread_id: "thread-1".to_owned(),
        message: "original".to_owned(),
        workspace_dir: Some("/tmp/work".to_owned()),
        images: Some(vec![ImagePayload {
            name: "image.png".to_owned(),
            data: "base64".to_owned(),
            media_type: "image/png".to_owned(),
        }]),
        metadata: std::collections::HashMap::from([(
            "bridge_run_id".to_owned(),
            Value::String("run-1".to_owned()),
        )]),
    };

    let continued = codex_timeout_auto_continue_options(&options);

    assert_eq!(continued.thread_id, "thread-1");
    assert_eq!(continued.message, "continue");
    assert_eq!(continued.workspace_dir.as_deref(), Some("/tmp/work"));
    assert!(continued.images.is_none());
    assert_eq!(
        continued
            .metadata
            .get("bridge_run_id")
            .and_then(Value::as_str),
        Some("run-1")
    );
    assert_eq!(
        continued
            .metadata
            .get(CODEX_TIMEOUT_AUTO_CONTINUE_METADATA_KEY)
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn test_extract_codex_thread_title_from_name_update() {
    let params = json!({
        "threadId": "thread-1",
        "threadName": "  Investigate   Codex app-server title events  "
    });

    assert_eq!(
        extract_codex_thread_title(&params).as_deref(),
        Some("Investigate Codex app-server title events")
    );
}

#[test]
fn test_extract_codex_thread_title_from_started_thread() {
    let params = json!({
        "thread": {
            "id": "thread-1",
            "name": "  Existing   Codex app-server title  "
        }
    });

    assert_eq!(
        extract_codex_thread_started_title(&params).as_deref(),
        Some("Existing Codex app-server title")
    );
}

#[test]
fn test_matches_turn_via_turn_object() {
    let params = json!({"turn": {"id": "u1"}});
    assert!(matches_turn(&params, "t1", "u1"));
    assert!(!matches_turn(&params, "t1", "u2"));
}

#[test]
fn test_extract_usage_full() {
    let turn = json!({
        "usage": {
            "inputTokens": 100,
            "outputTokens": 50,
            "totalCostUsd": 0.005,
        }
    });
    let (input, output, cost) = extract_usage(&turn);
    assert_eq!(input, 100);
    assert_eq!(output, 50);
    assert!((cost - 0.005).abs() < f64::EPSILON);
}

#[test]
fn test_extract_usage_snake_case() {
    let turn = json!({
        "usage": {
            "input_tokens": 200,
            "output_tokens": 80,
            "cost": 0.01,
        }
    });
    let (input, output, cost) = extract_usage(&turn);
    assert_eq!(input, 200);
    assert_eq!(output, 80);
    assert!((cost - 0.01).abs() < f64::EPSILON);
}

#[test]
fn test_extract_usage_missing() {
    let turn = json!({"status": "completed"});
    let (input, output, cost) = extract_usage(&turn);
    assert_eq!(input, 0);
    assert_eq!(output, 0);
    assert!((cost - 0.0).abs() < f64::EPSILON);
}

#[test]
fn test_extract_usage_string_values() {
    let turn = json!({
        "usage": {
            "inputTokens": "150",
            "outputTokens": "75",
            "totalCostUsd": "0.003",
        }
    });
    let (input, output, cost) = extract_usage(&turn);
    assert_eq!(input, 150);
    assert_eq!(output, 75);
    assert!((cost - 0.003).abs() < f64::EPSILON);
}

#[test]
fn completed_only_sub_agent_activity_synthesizes_a_paired_tool_use_frame() {
    // App-server 0.144 emits `subAgentActivity` solely as `item/completed`
    // (codex event_mapping); channels render tool activity from the ToolUse
    // frame, so the provider must synthesize the pair.
    let item = json!({
        "type": "subAgentActivity",
        "id": "item-1",
        "kind": "started",
        "agentPath": "reviewer",
        "agentThreadId": "thr-child",
    });

    let mut started_ids = std::collections::HashSet::new();
    let messages = tool_session_messages_for_completed_item(&item, &mut started_ids);
    assert_eq!(
        messages.len(),
        2,
        "expected synthesized ToolUse + ToolResult"
    );
    assert_eq!(messages[0].role, ProviderMessageRole::ToolUse);
    assert_eq!(messages[0].tool_name.as_deref(), Some("subAgent:reviewer"));
    assert_eq!(messages[0].tool_use_id.as_deref(), Some("item-1"));
    assert_eq!(messages[1].role, ProviderMessageRole::ToolResult);
    assert_eq!(messages[1].tool_name.as_deref(), Some("subAgent:reviewer"));
    assert_eq!(messages[1].tool_use_id.as_deref(), Some("item-1"));

    // A duplicate completion must not re-synthesize the ToolUse frame.
    let duplicate = tool_session_messages_for_completed_item(&item, &mut started_ids);
    assert_eq!(duplicate.len(), 1);
}

#[test]
fn completed_item_with_prior_started_frame_stays_a_single_tool_result() {
    let item = json!({
        "type": "sleep",
        "id": "item-3",
        "durationMs": 1500,
    });

    let mut started_ids = std::collections::HashSet::new();
    let started = tool_session_message_for_started_item(&item, &mut started_ids)
        .expect("sleep maps at item/started");
    assert_eq!(started.tool_name.as_deref(), Some("sleep"));
    assert!(started_ids.contains("item-3"));

    let messages = tool_session_messages_for_completed_item(&item, &mut started_ids);
    assert_eq!(
        messages.len(),
        1,
        "started items complete without synthesis"
    );
    assert_eq!(messages[0].tool_name.as_deref(), Some("sleep"));
}

#[test]
fn sub_agent_activity_without_agent_path_uses_generic_name() {
    let item = json!({
        "type": "subAgentActivity",
        "id": "item-2",
        "kind": "interacted",
    });
    let mut started_ids = std::collections::HashSet::new();
    let messages = tool_session_messages_for_completed_item(&item, &mut started_ids);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].tool_name.as_deref(), Some("subAgent"));
}

#[test]
fn reasoning_items_stay_excluded_from_tool_frames() {
    let item = json!({
        "type": "reasoning",
        "id": "item-4",
    });
    assert!(build_tool_session_message(&item, false).is_none());
}

#[test]
fn advisory_notifications_render_messages_for_known_methods_only() {
    assert_eq!(
        codex_advisory_notification_message("warning", &json!({ "message": "quota nearly used" })),
        Some("quota nearly used".to_owned())
    );
    assert_eq!(
        codex_advisory_notification_message(
            "configWarning",
            &json!({ "summary": "bad key", "details": "unknown field", "path": "/tmp/config.toml" })
        ),
        Some("bad key (unknown field) [config: /tmp/config.toml]".to_owned())
    );
    assert_eq!(
        codex_advisory_notification_message(
            "deprecationNotice",
            &json!({ "summary": "old flag", "details": "use --new" })
        ),
        Some("old flag (use --new)".to_owned())
    );
    // Blank payloads and unrelated methods are not advisories.
    assert_eq!(
        codex_advisory_notification_message("warning", &json!({ "message": "  " })),
        None
    );
    assert_eq!(
        codex_advisory_notification_message("turn/completed", &json!({ "message": "x" })),
        None
    );
}

#[test]
fn extract_rerouted_model_reads_to_model() {
    assert_eq!(
        extract_rerouted_model(&json!({
            "fromModel": "gpt-5.6-sol",
            "toModel": "gpt-5.6-luna",
            "reason": "capacity",
        })),
        Some("gpt-5.6-luna".to_owned())
    );
    assert_eq!(extract_rerouted_model(&json!({ "toModel": "  " })), None);
    assert_eq!(extract_rerouted_model(&json!({})), None);
}

fn token_usage_params(turn: &str, last: (i64, i64), total: (i64, i64)) -> serde_json::Value {
    json!({
        "threadId": "thr-1",
        "turnId": turn,
        "tokenUsage": {
            "last": { "inputTokens": last.0, "outputTokens": last.1 },
            "total": { "inputTokens": total.0, "outputTokens": total.1 },
            "modelContextWindow": 372000,
        }
    })
}

#[test]
fn turn_usage_fresh_thread_single_request_uses_totals() {
    let mut tracker = CodexTurnUsageTracker::default();
    assert!(tracker.observe(
        &token_usage_params("turn-1", (1200, 340), (1200, 340)),
        "thr-1",
        "turn-1"
    ));
    assert_eq!(tracker.finish(None, false), (1200, 340));
    assert_eq!(tracker.latest_totals(), Some((1200, 340)));
}

#[test]
fn turn_usage_resumed_thread_derives_from_replay_baseline() {
    // Resume replay emits the prior turn's snapshot (old turnId) first, then
    // this turn performs two requests.
    let mut tracker = CodexTurnUsageTracker::default();
    tracker.observe(
        &token_usage_params("turn-old", (900, 100), (50000, 10000)),
        "thr-1",
        "turn-2",
    );
    tracker.observe(
        &token_usage_params("turn-2", (1200, 340), (51200, 10340)),
        "thr-1",
        "turn-2",
    );
    tracker.observe(
        &token_usage_params("turn-2", (800, 60), (52000, 10400)),
        "thr-1",
        "turn-2",
    );
    assert_eq!(tracker.finish(None, true), (2000, 400));
}

#[test]
fn turn_usage_stale_snapshot_reissued_under_current_turn_counts_zero() {
    // Reviewer repro (#TASK-2058): a usage-limit failure right after resume
    // re-sends the unchanged prior-turn snapshot with the *current* turnId.
    // Totals have not moved from the replayed baseline, so the turn used zero.
    let mut tracker = CodexTurnUsageTracker::default();
    tracker.observe(
        &token_usage_params("turn-old", (1200, 340), (51200, 10340)),
        "thr-1",
        "turn-2",
    );
    tracker.observe(
        &token_usage_params("turn-2", (1200, 340), (51200, 10340)),
        "thr-1",
        "turn-2",
    );
    assert_eq!(tracker.finish(None, true), (0, 0));
}

#[test]
fn turn_usage_stale_snapshot_counts_zero_against_stored_baseline_too() {
    // Same repro but without a replay snapshot: the previous in-process turn
    // stored the thread's totals as the baseline.
    let mut tracker = CodexTurnUsageTracker::default();
    tracker.observe(
        &token_usage_params("turn-2", (1200, 340), (51200, 10340)),
        "thr-1",
        "turn-2",
    );
    assert_eq!(tracker.finish(Some((51200, 10340)), true), (0, 0));
}

#[test]
fn turn_usage_second_in_process_turn_uses_stored_baseline() {
    // Turn 2 on the same in-process thread: no replay, baseline comes from the
    // totals remembered when turn 1 finished.
    let mut tracker = CodexTurnUsageTracker::default();
    tracker.observe(
        &token_usage_params("turn-2", (700, 90), (51900, 10430)),
        "thr-1",
        "turn-2",
    );
    assert_eq!(tracker.finish(Some((51200, 10340)), true), (700, 90));
}

#[test]
fn turn_usage_repeated_same_request_snapshots_add_nothing() {
    let mut tracker = CodexTurnUsageTracker::default();
    for _ in 0..3 {
        tracker.observe(
            &token_usage_params("turn-1", (1200, 340), (1200, 340)),
            "thr-1",
            "turn-1",
        );
    }
    assert_eq!(tracker.finish(None, false), (1200, 340));
}

#[test]
fn turn_usage_resumed_thread_without_any_baseline_falls_back_to_last_plus_growth() {
    let mut tracker = CodexTurnUsageTracker::default();
    tracker.observe(
        &token_usage_params("turn-2", (1200, 340), (51200, 10340)),
        "thr-1",
        "turn-2",
    );
    tracker.observe(
        &token_usage_params("turn-2", (800, 60), (52000, 10400)),
        "thr-1",
        "turn-2",
    );
    assert_eq!(tracker.finish(None, true), (2000, 400));
}

#[test]
fn turn_usage_ignores_other_threads_and_reports_zero_without_snapshots() {
    let mut tracker = CodexTurnUsageTracker::default();
    assert!(!tracker.observe(
        &token_usage_params("turn-1", (1200, 340), (1200, 340)),
        "thr-other",
        "turn-1"
    ));
    assert_eq!(tracker.finish(None, false), (0, 0));
    assert_eq!(tracker.latest_totals(), None);
}

#[test]
fn test_resolve_runtime_codex_env_merges_provider_env() {
    let config = CodexAppServerConfig {
        env: HashMap::from([
            ("OPENAI_API_KEY".to_owned(), "from-config".to_owned()),
            (
                "OPENAI_BASE_URL".to_owned(),
                "https://example.test".to_owned(),
            ),
        ]),
        ..Default::default()
    };
    let metadata = HashMap::from([(
        "provider_env".to_owned(),
        json!({
            "OPENAI_API_KEY": "from-provider",
            "OPENAI_ORG_ID": "org_123",
        }),
    )]);

    let env = resolve_runtime_codex_env(&config, &metadata);
    assert_eq!(
        env.get("OPENAI_API_KEY").map(String::as_str),
        Some("from-provider")
    );
    assert_eq!(
        env.get("OPENAI_BASE_URL").map(String::as_str),
        Some("https://example.test")
    );
    assert_eq!(
        env.get("OPENAI_ORG_ID").map(String::as_str),
        Some("org_123")
    );
}

#[test]
fn test_resolve_runtime_codex_env_applies_provider_env_for_traex() {
    let config = CodexAppServerConfig {
        provider_type: ProviderType::Traex,
        env: HashMap::from([("TRAE_FROM_CONFIG".to_owned(), "keep".to_owned())]),
        ..Default::default()
    };
    let metadata = HashMap::from([(
        "provider_env".to_owned(),
        json!({
            "OPENAI_API_KEY": "",
            "OPENAI_ORG_ID": "org_123",
        }),
    )]);

    let env = resolve_runtime_codex_env(&config, &metadata);
    assert_eq!(
        env.get("TRAE_FROM_CONFIG").map(String::as_str),
        Some("keep")
    );
    assert_eq!(env.get("OPENAI_API_KEY").map(String::as_str), Some(""));
    assert_eq!(
        env.get("OPENAI_ORG_ID").map(String::as_str),
        Some("org_123")
    );
}

#[test]
fn test_resolve_runtime_codex_env_keeps_blank_provider_api_key_override() {
    let config = CodexAppServerConfig {
        env: HashMap::from([("OPENAI_API_KEY".to_owned(), "from-config".to_owned())]),
        ..Default::default()
    };
    let metadata = HashMap::from([(
        "provider_env".to_owned(),
        json!({
            "OPENAI_API_KEY": "",
        }),
    )]);

    let env = resolve_runtime_codex_env(&config, &metadata);
    assert_eq!(env.get("OPENAI_API_KEY").map(String::as_str), Some(""));
}

#[test]
fn test_resolve_runtime_codex_env_exports_task_cli_env() {
    let config = CodexAppServerConfig::default();
    let metadata = HashMap::from([
        ("agent_id".to_owned(), json!("codex")),
        (
            "runtime_context".to_owned(),
            json!({
                "thread_id": "thread::task",
                "task": {
                    "task_id": "#TASK-4",
                    "status": "todo",
                    "scope": "telegram/codex_bot"
                }
            }),
        ),
    ]);

    let env = resolve_runtime_codex_env(&config, &metadata);

    assert_eq!(
        env.get("GARYX_THREAD_ID").map(String::as_str),
        Some("thread::task")
    );
    assert_eq!(
        env.get("GARYX_ACTOR").map(String::as_str),
        Some("agent:codex")
    );
    assert_eq!(
        env.get("GARYX_TASK_ID").map(String::as_str),
        Some("#TASK-4")
    );
}

#[test]
fn test_codex_client_reuse_keeps_active_client_when_env_changes() {
    let existing = HashMap::from([("GARYX_THREAD_ID".to_owned(), "thread::old".to_owned())]);
    let desired = HashMap::from([("GARYX_THREAD_ID".to_owned(), "thread::new".to_owned())]);

    assert_eq!(
        decide_codex_client_reuse(&existing, &desired, 1),
        CodexClientReuseDecision::Reuse
    );
}

#[test]
fn test_codex_client_reuse_replaces_idle_client_when_env_changes() {
    let existing = HashMap::from([("GARYX_THREAD_ID".to_owned(), "thread::old".to_owned())]);
    let desired = HashMap::from([("GARYX_THREAD_ID".to_owned(), "thread::new".to_owned())]);

    assert_eq!(
        decide_codex_client_reuse(&existing, &desired, 0),
        CodexClientReuseDecision::ReplaceIdle
    );
}

#[test]
fn test_codex_client_idle_ttl_is_three_minutes() {
    assert_eq!(CODEX_CLIENT_IDLE_TTL, Duration::from_secs(180));
}

#[test]
fn test_build_input_items_text_only() {
    let options = ProviderRunOptions {
        thread_id: "s1".to_owned(),
        message: "hello world".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };
    let items = build_input_items(&options, false);
    assert_eq!(items.len(), 1);
    assert!(matches!(&items[0], InputItem::Text { text } if text == "hello world"));
}

#[test]
fn test_build_input_items_skips_agent_memory_for_builtin_codex() {
    let options = ProviderRunOptions {
        thread_id: "s1".to_owned(),
        message: "hello world".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([("agent_id".to_owned(), json!("codex"))]),
    };
    let items = build_input_items(&options, true);
    assert_eq!(items.len(), 1);
    assert!(matches!(&items[0], InputItem::Text { text } if text == "hello world"));
}

#[test]
fn test_build_input_items_prepends_memory_for_custom_agents() {
    let options = ProviderRunOptions {
        thread_id: "s1".to_owned(),
        message: "hello world".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([("agent_id".to_owned(), json!("reviewer"))]),
    };
    let items = build_input_items(&options, true);
    assert_eq!(items.len(), 1);
    assert!(
        matches!(&items[0], InputItem::Text { text } if text.starts_with("<garyx_memory_context>") && text.contains("<agent_memory agent_id=\"reviewer\"") && text.ends_with("hello world"))
    );
}

#[test]
fn test_build_input_items_does_not_append_task_status_suffix() {
    let options = ProviderRunOptions {
        thread_id: "s1".to_owned(),
        message: "继续".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([(
            "runtime_context".to_owned(),
            json!({
                "task": {
                    "task_id": "#TASK-4",
                    "status": "in_progress",
                    "assignee": { "kind": "agent", "agent_id": "codex" }
                }
            }),
        )]),
    };
    let items = build_input_items(&options, false);

    assert_eq!(items.len(), 1);
    assert!(matches!(&items[0], InputItem::Text { text } if text == "继续"));

    let items = build_input_items(&options, true);
    assert_eq!(items.len(), 1);
    match &items[0] {
        InputItem::Text { text } => {
            assert!(text.starts_with("<garyx_thread_metadata>"));
            assert!(text.contains("task_id: #TASK-4"));
            assert!(!text.contains("status=in_progress"));
            assert!(!text.contains("assignee=agent:codex"));
            assert!(text.ends_with("继续"));
        }
        _ => panic!("expected text input"),
    }
}

#[test]
fn test_build_input_items_with_images() {
    let img = ImagePayload {
        name: "sample.png".to_owned(),
        media_type: "image/png".to_owned(),
        data: "abc123==".to_owned(),
    };

    let options = ProviderRunOptions {
        thread_id: "s1".to_owned(),
        message: "analyze this".to_owned(),
        workspace_dir: None,
        images: Some(vec![img]),
        metadata: HashMap::new(),
    };
    let items = build_input_items(&options, false);
    assert_eq!(items.len(), 2);
    assert!(
        matches!(&items[1], InputItem::Image { url } if url == "data:image/png;base64,abc123==")
    );
}

#[test]
fn test_build_input_items_empty_image_data_skipped() {
    let img = ImagePayload {
        name: "empty.png".to_owned(),
        media_type: "image/png".to_owned(),
        data: String::new(),
    };

    let options = ProviderRunOptions {
        thread_id: "s1".to_owned(),
        message: "msg".to_owned(),
        workspace_dir: None,
        images: Some(vec![img]),
        metadata: HashMap::new(),
    };
    let items = build_input_items(&options, false);
    assert_eq!(items.len(), 1); // image skipped
}

#[test]
fn test_build_tool_session_message_command() {
    let item = json!({
        "type": "commandExecution",
        "id": "cmd_1",
        "status": "completed",
        "command": "ls -la"
    });
    let msg = build_tool_session_message(&item, true).unwrap();
    assert_eq!(msg.role_str(), "tool_result");
    assert_eq!(msg.tool_name.as_deref(), Some("commandExecution"));
    assert_eq!(msg.tool_use_id.as_deref(), Some("cmd_1"));
    assert_eq!(msg.is_error, Some(false));
}

#[test]
fn test_build_tool_session_message_failed() {
    let item = json!({
        "type": "commandExecution",
        "id": "cmd_2",
        "status": "failed",
    });
    let msg = build_tool_session_message(&item, true).unwrap();
    assert_eq!(msg.is_error, Some(true));
}

#[test]
fn test_build_tool_session_message_mcp() {
    let item = json!({
        "type": "mcpToolCall",
        "id": "mcp_1",
        "server": "filesystem",
        "tool": "read_file",
    });
    let msg = build_tool_session_message(&item, false).unwrap();
    assert_eq!(msg.role_str(), "tool_use");
    assert_eq!(msg.tool_name.as_deref(), Some("mcp:filesystem:read_file"));
    assert_eq!(msg.is_error, None);
}

#[test]
fn test_build_tool_session_message_skips_reasoning_items() {
    let item = json!({
        "type": "reasoning",
        "id": "reason_1",
        "summary": ["checking state"],
        "content": [],
    });

    assert!(build_tool_session_message(&item, false).is_none());
    assert!(build_tool_session_message(&item, true).is_none());
}

#[test]
fn test_build_tool_session_message_keeps_codex_tool_activity_records() {
    let command = json!({
        "type": "commandExecution",
        "id": "cmd_1",
        "status": "running",
        "command": "ls -la",
    });
    let command_use = build_tool_session_message(&command, false).unwrap();
    assert_eq!(command_use.role_str(), "tool_use");
    assert_eq!(command_use.tool_name.as_deref(), Some("commandExecution"));
    assert_eq!(command_use.tool_use_id.as_deref(), Some("cmd_1"));
    assert_eq!(
        command_use
            .metadata
            .get("item_type")
            .and_then(Value::as_str),
        Some("commandExecution")
    );

    let file_change = json!({
        "type": "fileChange",
        "id": "file_1",
        "status": "completed",
        "path": "src/lib.rs",
    });
    let file_result = build_tool_session_message(&file_change, true).unwrap();
    assert_eq!(file_result.role_str(), "tool_result");
    assert_eq!(file_result.tool_name.as_deref(), Some("fileChange"));
    assert_eq!(file_result.tool_use_id.as_deref(), Some("file_1"));
    assert_eq!(
        file_result
            .metadata
            .get("item_type")
            .and_then(Value::as_str),
        Some("fileChange")
    );
}

#[test]
fn test_reasoning_items_do_not_emit_tool_stream_events() {
    let emitted = std::sync::Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let emitted_cb = emitted.clone();
    let callback: StreamCallback = Box::new(move |event| {
        emitted_cb
            .lock()
            .expect("events mutex poisoned")
            .push(event);
    });
    let item = json!({
        "type": "reasoning",
        "id": "reason_1",
        "summary": ["checking state"],
    });

    for is_completed in [false, true] {
        if let Some(message) = build_tool_session_message(&item, is_completed) {
            emit_tool_stream_event(&message, callback.as_ref());
        }
    }

    assert!(emitted.lock().expect("events mutex poisoned").is_empty());
}

#[test]
fn test_build_tool_session_message_codex_schema_tool_types() {
    let cases = [
        (
            json!({
                "type": "hookPrompt",
                "id": "hook_1",
                "fragments": [],
            }),
            "hookPrompt",
        ),
        (
            json!({
                "type": "plan",
                "id": "plan_1",
                "text": "1. inspect\n2. patch",
            }),
            "plan",
        ),
        (
            json!({
                "type": "dynamicToolCall",
                "id": "dyn_1",
                "namespace": "image_gen",
                "tool": "generate",
                "status": "inProgress",
            }),
            "image_gen:generate",
        ),
        (
            json!({
                "type": "collabAgentToolCall",
                "id": "agent_1",
                "tool": "spawnAgent",
                "status": "inProgress",
            }),
            "spawnAgent",
        ),
        (
            json!({
                "type": "webSearch",
                "id": "web_1",
                "query": "codex app server schema",
            }),
            "webSearch",
        ),
        (
            json!({
                "type": "imageView",
                "id": "view_1",
                "path": "/tmp/probe.png",
            }),
            "imageView",
        ),
        (
            json!({
                "type": "imageGeneration",
                "id": "img_1",
                "status": "in_progress",
                "revisedPrompt": null,
                "result": "",
            }),
            "imageGeneration",
        ),
        (
            json!({
                "type": "enteredReviewMode",
                "id": "review_1",
                "review": "code review",
            }),
            "enteredReviewMode",
        ),
        (
            json!({
                "type": "exitedReviewMode",
                "id": "review_2",
                "review": "code review",
            }),
            "exitedReviewMode",
        ),
        (
            json!({
                "type": "contextCompaction",
                "id": "compact_1",
            }),
            "contextCompaction",
        ),
    ];

    for (item, expected_name) in cases {
        let msg = build_tool_session_message(&item, false).unwrap();
        assert_eq!(msg.role_str(), "tool_use");
        assert_eq!(msg.tool_name.as_deref(), Some(expected_name));
        assert_eq!(
            msg.metadata.get("source").and_then(Value::as_str),
            Some("codex_app_server")
        );
        assert_eq!(
            msg.metadata.get("item_type").and_then(Value::as_str),
            item.get("type").and_then(Value::as_str)
        );
    }
}

#[test]
fn test_build_tool_session_message_error_statuses() {
    for status in ["failed", "declined", "error", "canceled", "cancelled"] {
        let item = json!({
            "type": "dynamicToolCall",
            "id": format!("dyn_{status}"),
            "tool": "run",
            "status": status,
        });
        let msg = build_tool_session_message(&item, true).unwrap();
        assert_eq!(msg.role_str(), "tool_result");
        assert_eq!(msg.is_error, Some(true), "status {status} should be error");
    }

    let item = json!({
        "type": "dynamicToolCall",
        "id": "dyn_success_false",
        "tool": "run",
        "status": "completed",
        "success": false,
    });
    let msg = build_tool_session_message(&item, true).unwrap();
    assert_eq!(msg.is_error, Some(true));
}

#[test]
fn test_build_tool_session_message_irrelevant_type() {
    let item = json!({"type": "text", "text": "hello"});
    assert!(build_tool_session_message(&item, false).is_none());
}

#[test]
fn test_is_agent_message_item_matches_legacy_and_v2_shapes() {
    assert!(is_agent_message_item(&json!({
        "type": "agentMessage",
        "id": "msg-1",
        "text": "commentary"
    })));
    assert!(is_agent_message_item(&json!({
        "type": "AgentMessage",
        "id": "msg-2",
        "content": []
    })));
    assert!(!is_agent_message_item(&json!({
        "type": "commandExecution",
        "id": "cmd-1"
    })));
}

#[test]
fn test_is_user_message_item_matches_v2_shape() {
    assert!(is_user_message_item(&json!({
        "type": "userMessage",
        "id": "user-1",
        "content": [{"type": "text", "text": "hello"}]
    })));
    assert!(is_user_message_item(&json!({
        "type": "UserMessage",
        "id": "user-2",
        "content": []
    })));
    assert!(!is_user_message_item(&json!({
        "type": "agentMessage",
        "id": "msg-1"
    })));
}

#[test]
fn test_is_tool_activity_item_matches_supported_types() {
    assert!(is_tool_activity_item(&json!({
        "type": "hookPrompt",
        "id": "hook-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "plan",
        "id": "plan-1"
    })));
    assert!(!is_tool_activity_item(&json!({
        "type": "reasoning",
        "id": "reasoning-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "commandExecution",
        "id": "cmd-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "fileChange",
        "id": "file-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "mcpToolCall",
        "id": "mcp-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "dynamicToolCall",
        "id": "dyn-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "collabAgentToolCall",
        "id": "agent-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "webSearch",
        "id": "web-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "imageView",
        "id": "view-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "imageGeneration",
        "id": "img-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "enteredReviewMode",
        "id": "review-1"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "exitedReviewMode",
        "id": "review-2"
    })));
    assert!(is_tool_activity_item(&json!({
        "type": "contextCompaction",
        "id": "compact-1"
    })));
    assert!(!is_tool_activity_item(&json!({
        "type": "agentMessage",
        "id": "msg-1"
    })));
    assert!(!is_tool_activity_item(&json!({
        "type": "userMessage",
        "id": "user-1"
    })));
    assert!(!is_tool_activity_item(&json!({
        "type": "text",
        "text": "hello"
    })));
}

#[test]
fn test_agent_message_item_switch_with_tool_activity_inserts_separator() {
    let emitted = std::sync::Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let emitted_cb = emitted.clone();
    let callback: StreamCallback = Box::new(move |event| {
        emitted_cb
            .lock()
            .expect("events mutex poisoned")
            .push(event);
    });

    let mut current_item_id = None;
    let mut current_item_has_text = false;
    let mut response_parts = Vec::new();

    maybe_emit_agent_message_separator(
        Some("commentary-1"),
        &mut current_item_id,
        &mut current_item_has_text,
        &mut response_parts,
        callback.as_ref(),
    );
    assert_eq!(current_item_id.as_deref(), Some("commentary-1"));
    current_item_has_text = true;

    maybe_emit_agent_message_separator(
        Some("final-1"),
        &mut current_item_id,
        &mut current_item_has_text,
        &mut response_parts,
        callback.as_ref(),
    );

    assert_eq!(current_item_id.as_deref(), Some("final-1"));
    assert!(!current_item_has_text);
    assert_eq!(response_parts, vec!["\n\n".to_owned()]);
    assert_eq!(
        emitted.lock().expect("events mutex poisoned").as_slice(),
        &[StreamEvent::Boundary {
            kind: StreamBoundaryKind::AssistantSegment,
            pending_input_id: None,
        }]
    );
}

#[test]
fn test_agent_message_item_switch_without_tool_activity_inserts_separator() {
    let emitted = std::sync::Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let emitted_cb = emitted.clone();
    let callback: StreamCallback = Box::new(move |event| {
        emitted_cb
            .lock()
            .expect("events mutex poisoned")
            .push(event);
    });

    let mut current_item_id = Some("commentary-1".to_owned());
    let mut current_item_has_text = true;
    let mut response_parts = Vec::new();

    maybe_emit_agent_message_separator(
        Some("final-1"),
        &mut current_item_id,
        &mut current_item_has_text,
        &mut response_parts,
        callback.as_ref(),
    );

    assert_eq!(current_item_id.as_deref(), Some("final-1"));
    assert!(!current_item_has_text);
    assert_eq!(response_parts, vec!["\n\n".to_owned()]);
    assert_eq!(
        emitted.lock().expect("events mutex poisoned").as_slice(),
        &[StreamEvent::Boundary {
            kind: StreamBoundaryKind::AssistantSegment,
            pending_input_id: None,
        }]
    );
}

#[test]
fn test_agent_message_item_switch_without_prior_text_does_not_insert_separator() {
    let emitted = std::sync::Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let emitted_cb = emitted.clone();
    let callback: StreamCallback = Box::new(move |event| {
        emitted_cb
            .lock()
            .expect("events mutex poisoned")
            .push(event);
    });

    let mut current_item_id = Some("commentary-1".to_owned());
    let mut current_item_has_text = false;
    let mut response_parts = Vec::new();

    maybe_emit_agent_message_separator(
        Some("final-1"),
        &mut current_item_id,
        &mut current_item_has_text,
        &mut response_parts,
        callback.as_ref(),
    );

    assert_eq!(current_item_id.as_deref(), Some("final-1"));
    assert!(!current_item_has_text);
    assert!(response_parts.is_empty());
    assert!(emitted.lock().expect("events mutex poisoned").is_empty());
}

#[test]
fn test_append_codex_assistant_session_message_groups_by_item_id() {
    let mut session_messages = Vec::new();

    append_codex_assistant_session_message(&mut session_messages, Some("item-1"), "在。");
    append_codex_assistant_session_message(&mut session_messages, Some("item-1"), "先执行 ls。");
    append_codex_assistant_session_message(&mut session_messages, Some("item-2"), "结果如下。");

    assert_eq!(session_messages.len(), 2);
    assert_eq!(session_messages[0].role_str(), "assistant");
    assert_eq!(session_messages[0].text.as_deref(), Some("在。先执行 ls。"));
    assert_eq!(
        session_messages[0]
            .metadata
            .get("item_id")
            .and_then(Value::as_str),
        Some("item-1")
    );
    assert_eq!(session_messages[1].text.as_deref(), Some("结果如下。"));
    assert_eq!(
        session_messages[1]
            .metadata
            .get("item_id")
            .and_then(Value::as_str),
        Some("item-2")
    );
}

#[test]
fn test_emit_tool_stream_event_maps_roles() {
    let emitted = std::sync::Arc::new(std::sync::Mutex::new(Vec::<StreamEvent>::new()));
    let emitted_cb = emitted.clone();
    let callback: StreamCallback = Box::new(move |event| {
        emitted_cb
            .lock()
            .expect("events mutex poisoned")
            .push(event);
    });

    let tool_use = ProviderMessage::tool_use(
        json!({"type": "commandExecution"}),
        Some("cmd-1".to_owned()),
        Some("commandExecution".to_owned()),
    );
    let tool_result = ProviderMessage::tool_result(
        json!({"type": "commandExecution"}),
        Some("cmd-1".to_owned()),
        Some("commandExecution".to_owned()),
        Some(false),
    );

    emit_tool_stream_event(&tool_use, callback.as_ref());
    emit_tool_stream_event(&tool_result, callback.as_ref());

    let events = emitted.lock().expect("events mutex poisoned").clone();
    assert!(
        matches!(&events[0], StreamEvent::ToolUse { message } if message.role_str() == "tool_use")
    );
    assert!(
        matches!(&events[1], StreamEvent::ToolResult { message } if message.role_str() == "tool_result")
    );
}

#[test]
fn test_build_thread_start_params_full() {
    let config = CodexAppServerConfig {
        workspace_dir: Some("/tmp/work".to_owned()),
        model: "o3-mini".to_owned(),
        model_reasoning_effort: "xhigh".to_owned(),
        model_service_tier: "priority".to_owned(),
        approval_policy: "never".to_owned(),
        sandbox_mode: "danger-full-access".to_owned(),
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let params = build_thread_start_params(&config, None, "thread::test", "run-1", &HashMap::new());
    assert_eq!(params.cwd.as_deref(), Some("/tmp/work"));
    assert_eq!(params.model.as_deref(), Some("o3-mini"));
    assert_eq!(params.model_reasoning_effort.as_deref(), Some("xhigh"));
    assert_eq!(params.service_tier.as_deref(), Some("priority"));
    assert_eq!(params.approval_policy.as_deref(), Some("never"));
    assert_eq!(params.sandbox.as_deref(), Some("danger-full-access"));
    let config = params.config.expect("thread config should exist");
    assert!(
        config
            .get("developer_instructions")
            .and_then(Value::as_str)
            .is_some()
    );
}

#[test]
fn test_build_thread_start_params_metadata_reasoning_effort_override() {
    let config = CodexAppServerConfig {
        model_reasoning_effort: "medium".to_owned(),
        ..Default::default()
    };
    let metadata = HashMap::from([("model_reasoning_effort".to_owned(), json!("xhigh"))]);

    let params = build_thread_start_params(&config, None, "thread::test", "run-1", &metadata);

    assert_eq!(params.model_reasoning_effort.as_deref(), Some("xhigh"));
}

#[test]
fn test_build_thread_start_params_metadata_service_tier_override() {
    let config = CodexAppServerConfig {
        model_service_tier: "standard".to_owned(),
        ..Default::default()
    };
    // The per-thread override (run metadata) wins over the provider default.
    let metadata = HashMap::from([("model_service_tier".to_owned(), json!("priority"))]);

    let params = build_thread_start_params(&config, None, "thread::test", "run-1", &metadata);

    assert_eq!(params.service_tier.as_deref(), Some("priority"));
}

#[test]
fn test_build_thread_start_params_fallback_model() {
    let config = CodexAppServerConfig {
        model: String::new(),
        default_model: "gpt-4o".to_owned(),
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let params = build_thread_start_params(&config, None, "thread::test", "run-1", &HashMap::new());
    assert_eq!(params.model.as_deref(), Some("gpt-4o"));
    assert!(params.model_reasoning_effort.is_none());
}

#[test]
fn test_build_thread_start_params_workspace_override_wins() {
    let config = CodexAppServerConfig {
        workspace_dir: Some("/tmp/from-config".to_owned()),
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let params = build_thread_start_params(
        &config,
        Some("/tmp/from-request"),
        "thread::test",
        "run-1",
        &HashMap::new(),
    );
    assert_eq!(params.cwd.as_deref(), Some("/tmp/from-request"));
}

#[test]
fn test_build_thread_start_params_no_model() {
    let config = CodexAppServerConfig {
        model: String::new(),
        default_model: String::new(),
        approval_policy: String::new(),
        sandbox_mode: String::new(),
        workspace_dir: None,
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let params = build_thread_start_params(&config, None, "thread::test", "run-1", &HashMap::new());
    assert!(params.model.is_none());
    assert!(params.model_reasoning_effort.is_none());
    assert!(params.cwd.is_none());
    assert!(params.approval_policy.is_none());
    assert!(params.sandbox.is_none());
}

#[test]
fn test_build_thread_start_params_prefers_metadata_model_override() {
    let config = CodexAppServerConfig {
        model: "gpt-5".to_owned(),
        default_model: "gpt-5-codex".to_owned(),
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let metadata = HashMap::from([("model".to_owned(), json!("o3"))]);
    let params = build_thread_start_params(&config, None, "thread::test", "run-1", &metadata);
    assert_eq!(params.model.as_deref(), Some("o3"));
}

#[test]
fn test_build_turn_start_options_omits_unconfigured_defaults() {
    let config = CodexAppServerConfig {
        model: String::new(),
        default_model: String::new(),
        model_reasoning_effort: String::new(),
        model_service_tier: String::new(),
        ..Default::default()
    };

    let options = build_turn_start_options(&config, &HashMap::new());

    assert!(options.model.is_none());
    assert!(options.effort.is_none());
    assert!(options.service_tier.is_none());
}

#[test]
fn test_build_turn_start_options_prefers_metadata_over_provider_config() {
    let config = CodexAppServerConfig {
        model: "gpt-5.5".to_owned(),
        default_model: "gpt-5.4".to_owned(),
        model_reasoning_effort: "xhigh".to_owned(),
        model_service_tier: "priority".to_owned(),
        ..Default::default()
    };
    let metadata = HashMap::from([
        ("model".to_owned(), json!("gpt-5.4")),
        ("model_reasoning_effort".to_owned(), json!("medium")),
        ("model_service_tier".to_owned(), json!("standard")),
    ]);

    let options = build_turn_start_options(&config, &metadata);

    assert_eq!(options.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(options.effort.as_deref(), Some("medium"));
    assert_eq!(options.service_tier.as_deref(), Some("standard"));
}

#[test]
fn test_build_turn_start_options_uses_provider_config() {
    let config = CodexAppServerConfig {
        model: String::new(),
        default_model: "gpt-5.5".to_owned(),
        model_reasoning_effort: "xhigh".to_owned(),
        model_service_tier: "priority".to_owned(),
        ..Default::default()
    };

    let options = build_turn_start_options(&config, &HashMap::new());

    assert_eq!(options.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(options.effort.as_deref(), Some("xhigh"));
    assert_eq!(options.service_tier.as_deref(), Some("priority"));
}

#[test]
fn test_resolve_codex_actual_model_prefers_explicit_sources() {
    let config = CodexAppServerConfig {
        model: "gpt-5".to_owned(),
        default_model: "gpt-4.1".to_owned(),
        ..Default::default()
    };
    let metadata = HashMap::from([("model".to_owned(), json!("o3"))]);
    assert_eq!(
        resolve_codex_actual_model_with_config_path(&config, &metadata, None).as_deref(),
        Some("o3")
    );

    let metadata = HashMap::new();
    assert_eq!(
        resolve_codex_actual_model_with_config_path(&config, &metadata, None).as_deref(),
        Some("gpt-5")
    );

    let config = CodexAppServerConfig {
        model: String::new(),
        default_model: "gpt-4.1".to_owned(),
        ..Default::default()
    };
    assert_eq!(
        resolve_codex_actual_model_with_config_path(&config, &metadata, None).as_deref(),
        Some("gpt-4.1")
    );
}

#[test]
fn test_resolve_codex_actual_model_reads_cli_default_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        "model = \"gpt-5.4\"\nmodel_reasoning_effort = \"xhigh\"\n",
    )
    .expect("write config");

    let config = CodexAppServerConfig {
        model: String::new(),
        default_model: String::new(),
        ..Default::default()
    };

    assert_eq!(
        resolve_codex_actual_model_with_config_path(&config, &HashMap::new(), Some(&config_path))
            .as_deref(),
        Some("gpt-5.4")
    );
}

#[test]
fn test_build_thread_start_params_injects_remote_mcp_servers() {
    let mut metadata = HashMap::new();
    metadata.insert(
        "remote_mcp_servers".to_owned(),
        json!({
            "proof": {
                "command": "python3",
                "args": ["proof_server.py"],
                "env": {"PROOF_TOKEN": "abc"},
                "working_dir": "/tmp/proof"
            },
            "garyx": {
                "type": "http",
                "url": "http://127.0.0.1:31337/mcp",
                "headers": {"X-Run-Id": "run-1"}
            }
        }),
    );

    let params = build_thread_start_params(
        &CodexAppServerConfig {
            mcp_base_url: String::new(),
            ..Default::default()
        },
        None,
        "thread::test",
        "run-1",
        &metadata,
    );
    let config = params.config.expect("thread config");
    assert_eq!(
        config["mcp_servers"]["proof"]["command"].as_str(),
        Some("python3")
    );
    assert_eq!(
        config["mcp_servers"]["proof"]["cwd"].as_str(),
        Some("/tmp/proof")
    );
    assert_eq!(
        config["mcp_servers"]["garyx"]["url"].as_str(),
        Some("http://127.0.0.1:31337/mcp")
    );
    assert_eq!(
        config["mcp_servers"]["garyx"]["http_headers"]["X-Run-Id"].as_str(),
        Some("run-1")
    );
}

#[test]
fn test_build_thread_start_params_injects_gary_developer_instructions() {
    let config = CodexAppServerConfig {
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let params = build_thread_start_params(&config, None, "thread::test", "run-1", &HashMap::new());
    let config = params.config.expect("thread config should exist");
    let developer_instructions = config
        .get("developer_instructions")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(developer_instructions.contains("Garyx runtime guidance:"));
    assert!(developer_instructions.contains("Self-evolution:"));
    assert!(developer_instructions.contains("~/.garyx/skills/<skill-id>/SKILL.md"));
    assert!(developer_instructions.contains("garyx task create"));
    assert!(developer_instructions.contains("garyx automation create"));
    assert!(!developer_instructions.contains("Global Memory"));
    assert!(!developer_instructions.contains("</garyx_memory_context>"));
    assert!(!developer_instructions.contains("Current runtime context:"));
    assert!(!developer_instructions.contains("thread_id: thread::test"));
}

#[test]
fn test_build_thread_start_params_merges_runtime_system_prompt() {
    let config = CodexAppServerConfig {
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let params = build_thread_start_params(
        &config,
        None,
        "thread::test",
        "run-1",
        &HashMap::from([(
            "system_prompt".to_owned(),
            Value::String("Use concise bullets.".to_owned()),
        )]),
    );
    let config = params.config.expect("thread config should exist");
    let developer_instructions = config
        .get("developer_instructions")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(developer_instructions.contains("Garyx runtime guidance:"));
    assert!(developer_instructions.contains("Use concise bullets."));
    assert!(!developer_instructions.contains("Global Memory"));
    assert!(!developer_instructions.contains("Current runtime context:"));
}

#[test]
fn test_build_thread_start_params_custom_agent_without_prompt_omits_developer_instructions() {
    let config = CodexAppServerConfig {
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let params = build_thread_start_params(
        &config,
        None,
        "thread::test",
        "run-1",
        &HashMap::from([("agent_id".to_owned(), Value::String("reviewer".to_owned()))]),
    );

    assert!(params.config.is_none());
}

#[test]
fn test_build_thread_start_params_custom_agent_blank_prompt_omits_developer_instructions() {
    let config = CodexAppServerConfig {
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let params = build_thread_start_params(
        &config,
        None,
        "thread::test",
        "run-1",
        &HashMap::from([
            ("agent_id".to_owned(), Value::String("reviewer".to_owned())),
            ("system_prompt".to_owned(), Value::String("   ".to_owned())),
        ]),
    );

    assert!(params.config.is_none());
}

#[test]
fn test_build_thread_start_params_custom_agent_without_prompt_keeps_mcp_config() {
    let config = CodexAppServerConfig {
        mcp_base_url: "http://127.0.0.1:31337".to_owned(),
        ..Default::default()
    };
    let params = build_thread_start_params(
        &config,
        None,
        "thread::test",
        "run-1",
        &HashMap::from([("agent_id".to_owned(), Value::String("reviewer".to_owned()))]),
    );
    let config = params.config.expect("mcp config should exist");

    assert!(config.get("developer_instructions").is_none());
    assert!(config.get("mcp_servers").is_some());
}

#[test]
fn test_build_thread_start_params_keeps_runtime_context_out_of_developer_instructions() {
    let config = CodexAppServerConfig {
        mcp_base_url: String::new(),
        ..Default::default()
    };
    let params = build_thread_start_params(
        &config,
        Some("/tmp/ws"),
        "thread::ctx",
        "run-1",
        &HashMap::from([(
            "runtime_context".to_owned(),
            json!({
                "channel": "macapp",
                "account_id": "main",
                "bot_id": "macapp:main",
                "task": {
                    "task_id": "#TASK-9",
                    "status": "todo"
                }
            }),
        )]),
    );
    let config = params.config.expect("thread config should exist");
    let developer_instructions = config
        .get("developer_instructions")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(developer_instructions.contains("System capabilities:"));
    assert!(!developer_instructions.contains("channel: macapp"));
    assert!(!developer_instructions.contains("task_id: #TASK-9"));
    assert!(!developer_instructions.contains("thread_id: thread::ctx"));
    assert!(!developer_instructions.contains("workspace_dir: /tmp/ws"));
}

#[test]
fn test_build_thread_start_params_injects_default_garyx_mcp_server() {
    let params = build_thread_start_params(
        &CodexAppServerConfig::default(),
        None,
        "thread::stop-loop",
        "run-stop",
        &HashMap::new(),
    );
    let config = params.config.expect("thread config");
    assert_eq!(
        config["mcp_servers"]["garyx"]["url"].as_str(),
        Some("http://127.0.0.1:31337/mcp/thread%3A%3Astop-loop/run-stop")
    );
    assert_eq!(
        config["mcp_servers"]["garyx"]["http_headers"]["X-Run-Id"].as_str(),
        Some("run-stop")
    );
    assert_eq!(
        config["mcp_servers"]["garyx"]["http_headers"]["X-Thread-Id"].as_str(),
        Some("thread::stop-loop")
    );
    assert_eq!(
        config["mcp_servers"]["garyx"]["http_headers"]["X-Session-Key"].as_str(),
        Some("thread::stop-loop")
    );
}

#[test]
fn test_build_thread_start_params_merges_garyx_mcp_headers_from_metadata() {
    let params = build_thread_start_params(
        &CodexAppServerConfig::default(),
        None,
        "thread::verify",
        "run-verify",
        &HashMap::from([(
            "garyx_mcp_headers".to_owned(),
            json!({
                "X-Gary-Test-Role": "verifier"
            }),
        )]),
    );
    let config = params.config.expect("thread config");
    assert_eq!(
        config["mcp_servers"]["garyx"]["http_headers"]["X-Gary-Test-Role"].as_str(),
        Some("verifier")
    );
}

#[test]
fn test_build_thread_start_params_builtin_garyx_overrides_runtime_entry() {
    let mut metadata = HashMap::new();
    metadata.insert(
        "remote_mcp_servers".to_owned(),
        json!({
            "proof": {
                "command": "python3",
                "args": ["proof_server.py"]
            },
            "garyx": {
                "type": "http",
                "url": "http://127.0.0.1:31337",
                "headers": {"X-Run-Id": "stale-run"}
            }
        }),
    );

    let params = build_thread_start_params(
        &CodexAppServerConfig::default(),
        None,
        "thread::test",
        "run-1",
        &metadata,
    );
    let config = params.config.expect("thread config");
    assert_eq!(
        config["mcp_servers"]["proof"]["command"].as_str(),
        Some("python3")
    );
    assert_eq!(
        config["mcp_servers"]["garyx"]["url"].as_str(),
        Some("http://127.0.0.1:31337/mcp/thread%3A%3Atest/run-1")
    );
    assert_eq!(
        config["mcp_servers"]["garyx"]["http_headers"]["X-Run-Id"].as_str(),
        Some("run-1")
    );
    assert_eq!(
        config["mcp_servers"]["garyx"]["http_headers"]["X-Thread-Id"].as_str(),
        Some("thread::test")
    );
}

#[test]
fn test_build_input_items_uses_native_skill_invocation() {
    let options = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "Use the skill.".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::from([(
            "slash_command_skill_id".to_owned(),
            Value::String("proof-skill".to_owned()),
        )]),
    };

    let items = build_input_items(&options, false);
    assert_eq!(items.len(), 1);
    match &items[0] {
        InputItem::Text { text } => assert_eq!(text, "/proof-skill\n\nUse the skill."),
        InputItem::Image { .. } => panic!("expected text item"),
    }
}

#[test]
fn test_provider_type() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    assert_eq!(provider.provider_type(), ProviderType::CodexAppServer);
}

#[test]
fn test_is_ready_before_init() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    assert!(!provider.is_ready());
}

#[tokio::test]
async fn test_run_returns_not_ready() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    let options = ProviderRunOptions {
        thread_id: "test".to_owned(),
        message: "hello".to_owned(),
        workspace_dir: None,
        images: None,
        metadata: HashMap::new(),
    };
    let noop: StreamCallback = Box::new(|_| {});
    let err = provider.run_streaming(&options, noop).await.unwrap_err();
    assert!(matches!(err, BridgeError::ProviderNotReady));
}

#[tokio::test]
async fn test_abort_no_active_run() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    assert!(!provider.abort("nonexistent").await);
}

#[tokio::test]
async fn test_abort_cleans_session_tracking_when_client_missing() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    provider.active_runs.lock().await.insert(
        "run_1".to_owned(),
        ActiveCodexRun {
            garyx_thread_id: "sess::1".to_owned(),
            codex_thread_id: "thread_1".to_owned(),
            turn_id: "turn_1".to_owned(),
        },
    );
    provider.active_session_turns.lock().await.insert(
        "sess::1".to_owned(),
        (
            "thread_1".to_owned(),
            "turn_1".to_owned(),
            "run_1".to_owned(),
        ),
    );
    provider
        .active_session_callbacks
        .lock()
        .await
        .insert("sess::1".to_owned(), ("run_1".to_owned(), Arc::new(|_| {})));

    assert!(!provider.abort("run_1").await);
    assert!(provider.active_runs.lock().await.get("run_1").is_none());
    assert!(
        provider
            .active_session_turns
            .lock()
            .await
            .get("sess::1")
            .is_none()
    );
    assert!(
        provider
            .active_session_callbacks
            .lock()
            .await
            .get("sess::1")
            .is_none()
    );
}

#[tokio::test]
async fn test_clear_session() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    provider
        .session_map
        .lock()
        .await
        .insert("sess::1".to_owned(), "thread_x".to_owned());
    provider.active_session_turns.lock().await.insert(
        "sess::1".to_owned(),
        (
            "thread_x".to_owned(),
            "turn_1".to_owned(),
            "run_1".to_owned(),
        ),
    );
    provider
        .active_session_callbacks
        .lock()
        .await
        .insert("sess::1".to_owned(), ("run_1".to_owned(), Arc::new(|_| {})));

    assert!(provider.clear_session("sess::1").await);
    assert!(provider.session_map.lock().await.get("sess::1").is_none());
    assert!(
        provider
            .active_session_turns
            .lock()
            .await
            .get("sess::1")
            .is_none()
    );
    assert!(
        provider
            .active_session_callbacks
            .lock()
            .await
            .get("sess::1")
            .is_none()
    );
}

#[tokio::test]
async fn test_streaming_input_ack_waits_for_codex_user_message_item() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    let events = Arc::new(StdMutex::new(Vec::<StreamEvent>::new()));
    let captured_events = events.clone();
    let callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = Arc::new(move |event| {
        captured_events.lock().unwrap().push(event);
    });

    provider
        .active_session_callbacks
        .lock()
        .await
        .insert("thread::garyx".to_owned(), ("run_1".to_owned(), callback));
    provider.active_session_pending_acks.lock().await.insert(
        "thread::garyx".to_owned(),
        (
            "run_1".to_owned(),
            VecDeque::from([PendingCodexAckMarker::RootUserMessage]),
        ),
    );

    assert!(
        provider
            .enqueue_streaming_input_ack(
                "thread::garyx",
                "run_1",
                Some("queued_input:1".to_owned())
            )
            .await
    );

    assert!(
        events.lock().unwrap().is_empty(),
        "turn/steer acceptance should only enqueue; ACK is emitted by a later userMessage item"
    );
    assert!(
        !provider
            .acknowledge_next_codex_user_message("codex-thread-1", "run_1")
            .await,
        "callbacks are keyed by Garyx thread id, not Codex thread id"
    );
    assert!(
        !provider
            .acknowledge_next_codex_user_message("thread::garyx", "run_2")
            .await,
        "stale callbacks from another run must not receive acks"
    );
    assert!(
        !provider
            .acknowledge_next_codex_user_message("thread::garyx", "run_1")
            .await,
        "the first Codex userMessage item is the root prompt and must not ACK a queued follow-up"
    );
    assert!(events.lock().unwrap().is_empty());
    assert!(
        provider
            .acknowledge_next_codex_user_message("thread::garyx", "run_1")
            .await
    );

    let events = events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0],
        StreamEvent::Boundary {
            kind: StreamBoundaryKind::UserAck,
            pending_input_id: Some("queued_input:1".to_owned()),
        }
    );
}

#[tokio::test]
async fn test_get_or_create_session_existing() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    provider
        .session_map
        .lock()
        .await
        .insert("sess::a".to_owned(), "thread_abc".to_owned());

    let result = provider.get_or_create_session("sess::a").await.unwrap();
    assert_eq!(result, "thread_abc");
}

#[tokio::test]
async fn test_get_or_create_session_new() {
    let provider = CodexAgentProvider::new(CodexAppServerConfig::default());
    let result = provider.get_or_create_session("sess::new").await.unwrap();
    // Returns empty string as placeholder for new sessions
    assert!(result.is_empty());
}

#[test]
fn test_resolve_existing_thread_id_prefers_session_map() {
    let session_map = HashMap::from([("thread::one".to_owned(), "thread-from-memory".to_owned())]);

    let resolved =
        resolve_existing_thread_id(&session_map, "thread::one", Some("thread-from-persistence"));

    assert_eq!(resolved.as_deref(), Some("thread-from-memory"));
}

#[tokio::test]
async fn test_resume_or_start_thread_falls_back_to_start_after_resume_error() {
    let resume_calls = Arc::new(AtomicUsize::new(0));
    let start_calls = Arc::new(AtomicUsize::new(0));
    let thread_params = ThreadStartParams {
        cwd: Some("/tmp/workspace".to_owned()),
        config: None,
        model: Some("gpt-5".to_owned()),
        model_reasoning_effort: Some("xhigh".to_owned()),
        service_tier: None,
        approval_policy: Some("never".to_owned()),
        sandbox: Some("danger-full-access".to_owned()),
    };

    let thread_id = resume_or_start_thread(
        Some("stale-thread".to_owned()),
        false,
        thread_params.clone(),
        {
            let resume_calls = resume_calls.clone();
            move |params| {
                let resume_calls = resume_calls.clone();
                async move {
                    resume_calls.fetch_add(1, Ordering::Relaxed);
                    assert_eq!(params.thread_id, "stale-thread");
                    assert_eq!(params.cwd.as_deref(), Some("/tmp/workspace"));
                    Err(CodexError::RpcError {
                        code: -32600,
                        message: "no rollout found for thread id stale-thread".to_owned(),
                        data: None,
                    })
                }
            }
        },
        |_params| async move { Ok("unexpected-fork-thread".to_owned()) },
        {
            let start_calls = start_calls.clone();
            move |params| {
                let start_calls = start_calls.clone();
                async move {
                    start_calls.fetch_add(1, Ordering::Relaxed);
                    assert_eq!(params.cwd.as_deref(), Some("/tmp/workspace"));
                    assert_eq!(params.model.as_deref(), Some("gpt-5"));
                    Ok("fresh-thread".to_owned())
                }
            }
        },
    )
    .await
    .unwrap();

    assert_eq!(thread_id, "fresh-thread");
    assert_eq!(resume_calls.load(Ordering::Relaxed), 1);
    assert_eq!(start_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn test_resume_or_start_thread_uses_resumed_thread_without_starting_new_one() {
    let resume_calls = Arc::new(AtomicUsize::new(0));
    let start_calls = Arc::new(AtomicUsize::new(0));

    let thread_id = resume_or_start_thread(
        Some("existing-thread".to_owned()),
        false,
        ThreadStartParams::default(),
        {
            let resume_calls = resume_calls.clone();
            move |params| {
                let resume_calls = resume_calls.clone();
                async move {
                    resume_calls.fetch_add(1, Ordering::Relaxed);
                    assert_eq!(params.thread_id, "existing-thread");
                    Ok("existing-thread".to_owned())
                }
            }
        },
        |_params| async move { Ok("unexpected-fork-thread".to_owned()) },
        {
            let start_calls = start_calls.clone();
            move |_params| {
                let start_calls = start_calls.clone();
                async move {
                    start_calls.fetch_add(1, Ordering::Relaxed);
                    Ok("unexpected-new-thread".to_owned())
                }
            }
        },
    )
    .await
    .unwrap();

    assert_eq!(thread_id, "existing-thread");
    assert_eq!(resume_calls.load(Ordering::Relaxed), 1);
    assert_eq!(start_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn test_resume_or_start_thread_forks_existing_thread_without_resume_or_start() {
    let resume_calls = Arc::new(AtomicUsize::new(0));
    let fork_calls = Arc::new(AtomicUsize::new(0));
    let start_calls = Arc::new(AtomicUsize::new(0));

    let thread_id = resume_or_start_thread(
        Some("parent-thread".to_owned()),
        true,
        ThreadStartParams {
            cwd: Some("/tmp/workspace".to_owned()),
            config: Some(json!({"mcpServers": {}})),
            model: Some("gpt-5".to_owned()),
            model_reasoning_effort: Some("high".to_owned()),
            service_tier: None,
            approval_policy: Some("never".to_owned()),
            sandbox: Some("off".to_owned()),
        },
        {
            let resume_calls = resume_calls.clone();
            move |_params| {
                let resume_calls = resume_calls.clone();
                async move {
                    resume_calls.fetch_add(1, Ordering::Relaxed);
                    Ok("unexpected-resume-thread".to_owned())
                }
            }
        },
        {
            let fork_calls = fork_calls.clone();
            move |params| {
                let fork_calls = fork_calls.clone();
                async move {
                    fork_calls.fetch_add(1, Ordering::Relaxed);
                    assert_eq!(params.thread_id, "parent-thread");
                    assert_eq!(params.cwd.as_deref(), Some("/tmp/workspace"));
                    assert_eq!(params.model.as_deref(), Some("gpt-5"));
                    assert_eq!(params.model_reasoning_effort.as_deref(), Some("high"));
                    assert_eq!(params.approval_policy.as_deref(), Some("never"));
                    assert_eq!(params.sandbox.as_deref(), Some("off"));
                    Ok("forked-child-thread".to_owned())
                }
            }
        },
        {
            let start_calls = start_calls.clone();
            move |_params| {
                let start_calls = start_calls.clone();
                async move {
                    start_calls.fetch_add(1, Ordering::Relaxed);
                    Ok("unexpected-new-thread".to_owned())
                }
            }
        },
    )
    .await
    .unwrap();

    assert_eq!(thread_id, "forked-child-thread");
    assert_eq!(resume_calls.load(Ordering::Relaxed), 0);
    assert_eq!(fork_calls.load(Ordering::Relaxed), 1);
    assert_eq!(start_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn test_resume_or_start_thread_rejects_fork_without_parent_thread() {
    let err = resume_or_start_thread(
        None,
        true,
        ThreadStartParams::default(),
        |_params| async move { Ok("unexpected-resume-thread".to_owned()) },
        |_params| async move { Ok("unexpected-fork-thread".to_owned()) },
        |_params| async move { Ok("unexpected-new-thread".to_owned()) },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, BridgeError::SessionError(_)));
    assert!(err.to_string().contains("without parent thread id"));
}

#[test]
fn test_map_codex_error() {
    let err = map_codex_error("thread/start failed", CodexError::Fatal("boom".to_owned()));
    assert!(matches!(err, BridgeError::RunFailed(_)));
    assert!(err.to_string().contains("thread/start failed"));
    assert!(err.to_string().contains("boom"));
}

#[test]
fn test_cwd_canonicalization_resolves_dotdot() {
    // /tmp/../tmp should resolve; on macOS /tmp -> /private/tmp
    let input = "/tmp/../tmp";
    let canonical = std::fs::canonicalize(input)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| input.to_owned());
    let expected = std::fs::canonicalize("/tmp")
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(canonical, expected);
}

#[test]
fn test_cwd_canonicalization_fallback_for_nonexistent_path() {
    let bogus = "/nonexistent_path_abc123_xyz";
    let result = std::fs::canonicalize(bogus)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| bogus.to_owned());
    assert_eq!(result, bogus);
}

#[test]
fn test_normalize_codex_mcp_servers_canonicalizes_cwd() {
    let mut servers = serde_json::Map::new();
    servers.insert(
        "test-server".to_owned(),
        json!({
            "command": "node",
            "args": ["server.js"],
            "cwd": "/tmp/../tmp"
        }),
    );
    let mut metadata = HashMap::new();
    metadata.insert("remote_mcp_servers".to_owned(), Value::Object(servers));

    let result = normalize_codex_mcp_servers(&metadata).unwrap();
    let server = result.as_object().unwrap().get("test-server").unwrap();
    let cwd = server.get("cwd").unwrap().as_str().unwrap();
    let expected = std::fs::canonicalize("/tmp")
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(cwd, expected);
}
