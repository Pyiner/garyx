use super::*;
use axum::Router;
use axum::body::Body;
use axum::http::Request;
use garyx_models::config::GaryxConfig;
use tempfile::tempdir;
use tower::ServiceExt;

fn test_state() -> Arc<AppState> {
    use crate::composition::app_bootstrap::AppStateBuilder;
    // Use in-memory stores to avoid filesystem races between concurrent tests.
    AppStateBuilder::new(GaryxConfig::default())
        .with_auto_research_store(Arc::new(crate::auto_research::AutoResearchStore::new()))
        .with_agent_team_store(Arc::new(crate::agent_teams::AgentTeamStore::new()))
        .with_custom_agent_store(Arc::new(crate::custom_agents::CustomAgentStore::new()))
        .build()
}

async fn seed_transcript_backed_thread(state: &Arc<AppState>, thread_id: &str, mut data: Value) {
    let messages = data
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(obj) = data.as_object_mut() {
        obj.insert(
            "message_count".to_owned(),
            Value::Number(serde_json::Number::from(messages.len() as u64)),
        );
        obj.insert(
            "history".to_owned(),
            json!({
                "source": "transcript_v1",
                "message_count": messages.len(),
            }),
        );
    }
    state.threads.thread_store.set(thread_id, data).await;
    if !messages.is_empty() {
        state
            .threads
            .history
            .transcript_store()
            .rewrite_from_messages(thread_id, &messages)
            .await
            .unwrap();
    }
}

fn api_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/threads/history", axum::routing::get(thread_history))
        .route("/api/debug/thread", axum::routing::get(debug_thread))
        .route("/api/debug/bot", axum::routing::get(debug_bot))
        .route(
            "/api/debug/bot/threads",
            axum::routing::get(debug_bot_threads),
        )
        .route(
            "/api/auto-research/runs",
            axum::routing::post(create_auto_research_run),
        )
        .route(
            "/api/auto-research/runs/{run_id}",
            axum::routing::get(get_auto_research_run).delete(delete_auto_research_run),
        )
        .route(
            "/api/auto-research/runs/{run_id}/iterations",
            axum::routing::get(list_auto_research_iterations),
        )
        .route(
            "/api/auto-research/runs/{run_id}/stop",
            axum::routing::post(stop_auto_research_run),
        )
        .route(
            "/api/auto-research/runs/{run_id}/candidates",
            axum::routing::get(list_auto_research_candidates),
        )
        .route(
            "/api/auto-research/runs/{run_id}/select/{candidate_id}",
            axum::routing::post(select_auto_research_candidate),
        )
        .route(
            "/api/custom-agents",
            axum::routing::get(list_custom_agents).post(create_custom_agent),
        )
        .route(
            "/api/custom-agents/{agent_id}",
            axum::routing::get(get_custom_agent)
                .put(update_custom_agent)
                .delete(delete_custom_agent),
        )
        .route(
            "/api/teams",
            axum::routing::get(list_agent_teams).post(create_agent_team),
        )
        .route(
            "/api/teams/{team_id}",
            axum::routing::get(get_agent_team)
                .put(update_agent_team)
                .delete(delete_agent_team),
        )
        .route("/api/cron/jobs", axum::routing::get(cron_jobs))
        .route("/api/cron/runs", axum::routing::get(cron_runs))
        .route(
            "/api/heartbeat/summary",
            axum::routing::get(heartbeat_summary),
        )
        .route(
            "/api/heartbeat/trigger",
            axum::routing::post(heartbeat_trigger),
        )
        .route("/api/settings", axum::routing::put(settings_update))
        .route("/api/settings/reload", axum::routing::post(settings_reload))
        .route("/api/restart", axum::routing::post(restart))
        .with_state(state)
}

#[test]
fn test_stringify_message_content_summarizes_text_and_images() {
    let content = json!([
        {
            "type": "text",
            "text": "look at this"
        },
        {
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/png",
                "data": "abc123=="
            }
        }
    ]);

    assert_eq!(
        stringify_message_content(&content),
        "look at this\n\n[1 image]"
    );
}

#[test]
fn test_stringify_message_content_summarizes_image_only_payloads() {
    let content = json!([
        {
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/png",
                "data": "abc123=="
            }
        },
        {
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/jpeg",
                "data": "def456=="
            }
        }
    ]);

    assert_eq!(stringify_message_content(&content), "[2 images]");
}

#[tokio::test]
async fn test_thread_history_empty() {
    let state = test_state();
    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/threads/history")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["total"], 0);
    assert_eq!(json["limit"], 50);
    assert!(json["threads"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_thread_history_with_data() {
    let state = test_state();
    state
        .threads
        .thread_store
        .set("thread::agent1-user1", json!({"msg": "hello"}))
        .await;
    state
        .threads
        .thread_store
        .set("thread::agent1-user2", json!({"msg": "world"}))
        .await;

    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/threads/history?limit=1&include_messages=true")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["total"], 2);
    assert_eq!(json["limit"], 1);
    assert_eq!(json["threads"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_debug_thread_returns_ledger_records() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "thread::debug-alpha",
        json!({
            "messages": [{
                "role": "user",
                "content": "hello",
                "timestamp": "2026-03-22T10:00:00Z"
            }]
        }),
    )
    .await;
    state
        .threads
        .message_ledger
        .append_event(garyx_models::MessageLedgerEvent {
            ledger_id: "ledger-1".to_owned(),
            bot_id: "telegram:main".to_owned(),
            status: garyx_models::MessageLifecycleStatus::RunInterrupted,
            created_at: "2026-03-22T10:00:01Z".to_owned(),
            thread_id: Some("thread::debug-alpha".to_owned()),
            run_id: Some("run-1".to_owned()),
            channel: Some("telegram".to_owned()),
            account_id: Some("main".to_owned()),
            chat_id: Some("-100".to_owned()),
            from_id: Some("42".to_owned()),
            native_message_id: Some("tg-1".to_owned()),
            text_excerpt: Some("hello".to_owned()),
            terminal_reason: Some(garyx_models::MessageTerminalReason::SelfRestart),
            reply_message_id: None,
            metadata: json!({"reason":"restart"}),
        })
        .await
        .unwrap();
    state
        .threads
        .thread_store
        .set(
            "thread::debug-alpha",
            json!({
                "thread_id": "thread::debug-alpha",
                "provider_type": "claude_code",
                "sdk_session_id": "sdk-123",
                "history": {
                    "active_run_snapshot": {
                        "run_id": "run-1",
                        "provider_key": "claude_code:old-hash",
                        "updated_at": "2026-03-22T10:00:02Z",
                        "pending_user_inputs": [{"id":"q1"}]
                    }
                }
            }),
        )
        .await;

    let router = api_router(state);
    let req = Request::builder()
        .uri("/api/debug/thread?thread_id=thread::debug-alpha")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["thread_id"], "thread::debug-alpha");
    assert_eq!(json["thread_runtime"]["provider_type"], "claude_code");
    assert_eq!(json["thread_runtime"]["provider_label"], "Claude");
    assert_eq!(json["thread_runtime"]["sdk_session_id"], "sdk-123");
    assert!(json["thread_runtime"]["active_run"].is_null());
    assert!(json["thread"]["history"]["active_run_snapshot"].is_null());
    assert_eq!(
        json["message_ledger"]["records"][0]["terminal_reason"],
        "self_restart"
    );
}

#[tokio::test]
async fn test_debug_bot_threads_returns_problem_threads() {
    let state = test_state();
    state
        .threads
        .message_ledger
        .append_event(garyx_models::MessageLedgerEvent {
            ledger_id: "ledger-1".to_owned(),
            bot_id: "telegram:main".to_owned(),
            status: garyx_models::MessageLifecycleStatus::RunInterrupted,
            created_at: "2026-03-22T10:00:01Z".to_owned(),
            thread_id: Some("thread::debug-alpha".to_owned()),
            run_id: Some("run-1".to_owned()),
            channel: Some("telegram".to_owned()),
            account_id: Some("main".to_owned()),
            chat_id: Some("-100".to_owned()),
            from_id: Some("42".to_owned()),
            native_message_id: Some("tg-1".to_owned()),
            text_excerpt: Some("hello".to_owned()),
            terminal_reason: Some(garyx_models::MessageTerminalReason::SelfRestart),
            reply_message_id: None,
            metadata: json!({}),
        })
        .await
        .unwrap();

    let router = api_router(state);
    let req = Request::builder()
        .uri("/api/debug/bot/threads?bot_id=telegram:main")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["threads"][0]["thread_id"], "thread::debug-alpha");
    assert_eq!(json["threads"][0]["terminal_reason"], "self_restart");
}

#[tokio::test]
async fn test_create_auto_research_run() {
    let state = test_state();
    let router = api_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/auto-research/runs")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "goal": "Compare two options",
                "workspace_dir": "/tmp/example-workspace",
                "max_iterations": 10,
                "time_budget_secs": 60
            }))
            .unwrap(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(
        status,
        StatusCode::CREATED,
        "unexpected auto research create response: {}",
        String::from_utf8_lossy(&body)
    );
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["state"], "queued");
    assert_eq!(json["goal"], "Compare two options");
    assert_eq!(json["workspace_dir"], "/tmp/example-workspace");
    assert_eq!(json["max_iterations"], 10);
    assert!(json["state_started_at"].as_str().is_some());
    assert!(json["run_id"].as_str().unwrap().starts_with("ar_"));
}

#[tokio::test]
async fn test_get_auto_research_run_with_latest_iteration() {
    let state = test_state();
    let run = state
        .ops
        .auto_research
        .create_run(crate::auto_research::CreateAutoResearchRunRequest {
            goal: Some("Compare two options".to_owned()),
            workspace_dir: Some("/tmp/garyx".to_owned()),
            provider_metadata: std::collections::HashMap::new(),
            max_iterations: 10,
            time_budget_secs: 60,
            ..Default::default()
        })
        .await
        .unwrap();
    state
        .ops
        .auto_research
        .seed_iteration(
            &run.run_id,
            1,
            garyx_models::AutoResearchIterationState::Judging,
            Some("thread::auto-research::seeded::work::1".to_owned()),
            Some("thread::auto-research::seeded::verify::1".to_owned()),
        )
        .await
        .unwrap();

    let router = api_router(state);
    let req = Request::builder()
        .uri(format!("/api/auto-research/runs/{}", run.run_id))
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["run"]["run_id"], run.run_id);
    assert_eq!(json["latest_iteration"]["iteration_index"], 1);
    assert!(json["run"]["state_started_at"].as_str().is_some());
}

#[tokio::test]
async fn test_stop_auto_research_run() {
    let state = test_state();
    let run = state
        .ops
        .auto_research
        .create_run(crate::auto_research::CreateAutoResearchRunRequest {
            goal: Some("Compare two options".to_owned()),
            workspace_dir: Some("/tmp/garyx".to_owned()),
            provider_metadata: std::collections::HashMap::new(),
            max_iterations: 10,
            time_budget_secs: 60,
            ..Default::default()
        })
        .await
        .unwrap();
    let router = api_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/auto-research/runs/{}/stop", run.run_id))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({"reason":"user_requested"})).unwrap(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["state"], "user_stopped");
    assert_eq!(json["terminal_reason"], "user_requested");
}

#[tokio::test]
async fn test_create_and_list_custom_agents() {
    let state = test_state();
    let router = api_router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/api/custom-agents")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "agent_id": "spec-review",
                "display_name": "Spec Review",
                "role": "reviewer",
                "provider_type": "codex_app_server",
                "model": "gpt-5-codex",
                "system_prompt": "Review specs carefully."
            })
            .to_string(),
        ))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let req = Request::builder()
        .method("GET")
        .uri("/api/custom-agents")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["agents"].as_array().unwrap().iter().any(|agent| {
        agent["agent_id"] == "spec-review"
            && agent["provider_type"] == "codex_app_server"
            && agent["display_name"] == "Spec Review"
            && agent["model"] == "gpt-5-codex"
    }));
}

#[tokio::test]
async fn test_create_custom_agent_allows_omitted_model() {
    let state = test_state();
    let router = api_router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/api/custom-agents")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "agent_id": "plain-claude",
                "display_name": "Plain Claude",
                "provider_type": "claude_code",
                "system_prompt": "Work normally."
            })
            .to_string(),
        ))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["model"], "");
}

#[tokio::test]
async fn test_create_and_list_teams() {
    let state = test_state();
    let router = api_router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/api/teams")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "team_id": "product-ship",
                "display_name": "Product Ship",
                "leader_agent_id": "planner",
                "member_agent_ids": ["planner", "generator", "reviewer"],
                "workflow_text": "Leader plans, generator implements, reviewer validates."
            })
            .to_string(),
        ))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let req = Request::builder()
        .method("GET")
        .uri("/api/teams")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["teams"].as_array().unwrap().iter().any(|team| {
        team["team_id"] == "product-ship"
            && team["leader_agent_id"] == "planner"
            && team["display_name"] == "Product Ship"
    }));
}

#[tokio::test]
async fn test_update_team_prunes_removed_member_group_state() {
    use crate::agent_teams::UpsertAgentTeamRequest;
    use garyx_bridge::providers::agent_team::Group;

    let state = test_state();
    state
        .ops
        .agent_teams
        .upsert_team(UpsertAgentTeamRequest {
            team_id: "product-ship".to_owned(),
            display_name: "Product Ship".to_owned(),
            leader_agent_id: "planner".to_owned(),
            member_agent_ids: vec![
                "planner".to_owned(),
                "coder".to_owned(),
                "reviewer".to_owned(),
            ],
            workflow_text: "ship".to_owned(),
        })
        .await
        .expect("seed team");

    let thread_id = "thread::team-lifecycle";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "product-ship",
                "provider_type": "agent_team",
            }),
        )
        .await;

    let mut group = Group::new(thread_id, "product-ship");
    group.record_child_thread("coder", "th::child-coder");
    group.record_child_thread("reviewer", "th::child-reviewer");
    group.advance_catch_up("coder", 3);
    group.advance_catch_up("reviewer", 7);
    state.ops.agent_team_group_store.save(&group).await;

    let router = api_router(state.clone());
    let req = Request::builder()
        .method("PUT")
        .uri("/api/teams/product-ship")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "team_id": "product-ship",
                "display_name": "Product Ship",
                "leader_agent_id": "planner",
                "member_agent_ids": ["planner", "coder"],
                "workflow_text": "ship"
            })
            .to_string(),
        ))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let group = state
        .ops
        .agent_team_group_store
        .load(thread_id)
        .await
        .expect("group should remain");
    assert_eq!(
        group.child_threads.get("coder").map(String::as_str),
        Some("th::child-coder")
    );
    assert!(!group.child_threads.contains_key("reviewer"));
    assert_eq!(group.catch_up_offset("coder"), 3);
    assert!(!group.catch_up_offsets.contains_key("reviewer"));

    let team = state
        .ops
        .agent_teams
        .get_team("product-ship")
        .await
        .expect("team should remain in registry");
    assert_eq!(team.member_agent_ids, vec!["planner", "coder"]);
}

#[tokio::test]
async fn test_delete_team_marks_threads_deleted_and_drops_group_state() {
    use crate::agent_teams::UpsertAgentTeamRequest;
    use garyx_bridge::providers::agent_team::Group;

    let state = test_state();
    state
        .ops
        .agent_teams
        .upsert_team(UpsertAgentTeamRequest {
            team_id: "product-ship".to_owned(),
            display_name: "Product Ship".to_owned(),
            leader_agent_id: "planner".to_owned(),
            member_agent_ids: vec!["planner".to_owned(), "coder".to_owned()],
            workflow_text: "ship".to_owned(),
        })
        .await
        .expect("seed team");
    state
        .integration
        .bridge
        .replace_team_profiles(state.ops.agent_teams.list_teams().await)
        .await;

    let thread_id = "thread::deleted-team";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "product-ship",
                "provider_type": "agent_team",
            }),
        )
        .await;

    let mut group = Group::new(thread_id, "product-ship");
    group.record_child_thread("coder", "th::child-coder");
    state.ops.agent_team_group_store.save(&group).await;

    let router = api_router(state.clone());
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/teams/product-ship")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let thread = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("thread should remain for history");
    assert_eq!(thread["team_deleted"], true);
    assert_eq!(thread["team_deleted_id"], "product-ship");
    assert!(
        thread
            .get("team_deleted_at")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    );

    assert!(
        state
            .ops
            .agent_team_group_store
            .load(thread_id)
            .await
            .is_none(),
        "group state should be deleted with the team"
    );
    assert!(
        state
            .integration
            .bridge
            .team_profile("product-ship")
            .await
            .is_none(),
        "bridge registry should drop deleted team profile"
    );
}

#[tokio::test]
async fn test_create_team_accepts_canonical_payload() {
    let state = test_state();
    let router = api_router(state);
    let req = Request::builder()
        .method("POST")
        .uri("/api/teams")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "teamId": "product-ship-camel",
                "displayName": "Product Ship Camel",
                "leaderAgentId": "planner",
                "memberAgentIds": ["planner", "generator", "reviewer"],
                "workflowText": "Leader plans, generator implements, reviewer validates."
            })
            .to_string(),
        ))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["team_id"], "product-ship-camel");
    assert_eq!(json["display_name"], "Product Ship Camel");
}

#[tokio::test]
async fn test_create_auto_research_run_progresses_to_terminal_state() {
    let state = test_state();
    let router = api_router(state.clone());

    let req = Request::builder()
        .method("POST")
        .uri("/api/auto-research/runs")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "goal": "Compare two options",
                "workspace_dir": "/tmp/example-workspace",
                "max_iterations": 10,
                "time_budget_secs": 60
            }))
            .unwrap(),
        ))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let run_id = json["run_id"].as_str().unwrap().to_owned();

    // Poll until the run reaches a terminal state.
    // Use a generous timeout (8s) to avoid flaky failures under CI load.
    let mut terminal_state = None;
    for _ in 0..160 {
        if let Some(run) = state.ops.auto_research.get_run(&run_id).await {
            if run.state.is_terminal() {
                terminal_state = Some(run.state);
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // Without real provider keys, the run is immediately Blocked
    // (no scaffold fallback). Accept either terminal state.
    assert!(
        terminal_state.as_ref().is_some_and(|s| s.is_terminal()),
        "run should reach a terminal state within the polling window, got: {terminal_state:?}"
    );
    // Blocked runs (no provider keys) may have 0 iterations.
    // Only check iteration details if any exist.
    let iterations = state
        .ops
        .auto_research
        .list_iterations(&run_id)
        .await
        .unwrap_or_default();
    // iterations exist but content/verdict now lives on Candidate, not Iteration
    let _ = iterations;
}

#[tokio::test]
async fn test_thread_history_with_prefix() {
    let state = test_state();
    state
        .threads
        .thread_store
        .set("thread::agent1-user1", json!({"msg": "a"}))
        .await;
    state
        .threads
        .thread_store
        .set("thread::agent2-user2", json!({"msg": "b"}))
        .await;

    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/threads/history?prefix=thread::agent1")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["total"], 1);
}

#[tokio::test]
async fn test_thread_history_detail_with_thread_id_and_tool_messages() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "main::main::u1",
        json!({
            "messages": [
                {"role": "user", "content": "hello", "timestamp": "2026-03-01T00:00:00Z"},
                {"role": "assistant", "content": "world", "timestamp": "2026-03-01T00:00:01Z"}
            ],
            "outbound_message_ids": [
                {"channel": "telegram", "account_id": "main", "chat_id": "u1", "message_id": 123, "timestamp": "2026-03-01T00:00:02Z"}
            ],
            "channel": "telegram",
            "account_id": "main",
            "from_id": "u1"
        }),
    )
    .await;

    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/threads/history?thread_id=main%3A%3Amain%3A%3Au1&limit=10&include_tool_messages=true")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["thread"]["thread_id"], "main::main::u1");
    assert_eq!(json["thread"]["thread_key"], "main::main::u1");
    assert_eq!(json["thread"]["thread_type"], "chat");
    assert_eq!(json["session"]["thread_id"], "main::main::u1");
    assert_eq!(json["session"]["thread_key"], "main::main::u1");
    assert_eq!(json["session"]["thread_type"], "chat");
    assert_eq!(json["message_stats"]["total_messages_in_thread"], 2);
    assert_eq!(json["message_stats"]["total_messages_in_session"], 2);
    assert_eq!(json["message_stats"]["returned_messages"], 2);
    assert_eq!(json["messages"].as_array().unwrap().len(), 2);
    assert_eq!(json["outbound_total"], 1);
    assert_eq!(json["messages"][0]["text"], "hello");
    assert_eq!(json["messages"][0]["message"]["role"], "user");
    assert_eq!(json["messages"][1]["text"], "world");
    assert_eq!(json["messages"][1]["message"]["content"], "world");
}

#[tokio::test]
async fn test_thread_history_detail_filters_tool_messages() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "main::main::u2",
        json!({
            "messages": [
                {"role": "user", "content": "hi", "timestamp": "2026-03-01T00:00:00Z"},
                {"role": "tool_use", "content": {"tool_use_id": "tool_1"}, "timestamp": "2026-03-01T00:00:01Z"},
                {"role": "assistant", "content": "done", "timestamp": "2026-03-01T00:00:02Z"}
            ]
        }),
    )
    .await;

    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/threads/history?thread_id=main%3A%3Amain%3A%3Au2&include_tool_messages=false")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["message_stats"]["returned_messages"], 2);
    assert_eq!(
        json["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["tool_related"].as_bool().unwrap_or(false))
            .count(),
        0
    );
}

#[tokio::test]
async fn test_thread_history_detail_repairs_orphaned_pending_user_inputs() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "thread::pending-u3",
        json!({
            "messages": [
                {"role": "user", "content": "hello", "timestamp": "2026-03-01T00:00:00Z"}
            ],
            "pending_user_inputs": [
                {
                    "id": "pending-1",
                    "bridge_run_id": "run-not-active",
                    "text": "follow-up after reconnect",
                    "content": [{"type": "text", "text": "follow-up after reconnect"}],
                    "queued_at": "2026-03-01T00:00:05Z",
                    "status": "queued"
                }
            ]
        }),
    )
    .await;

    let router = api_router(state.clone());

    let req = Request::builder()
        .uri("/api/threads/history?thread_id=thread%3A%3Apending-u3&limit=10")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["pending_user_inputs"].as_array().unwrap().len(), 0);
    assert_eq!(json["message_stats"]["pending_user_input_count"], 0);
    assert_eq!(json["message_stats"]["active_pending_user_input_count"], 0);

    let repaired = state
        .threads
        .thread_store
        .get("thread::pending-u3")
        .await
        .expect("thread should still exist");
    assert_eq!(
        repaired["pending_user_inputs"]
            .as_array()
            .expect("pending_user_inputs should stay as an array")
            .len(),
        0
    );
}

#[tokio::test]
async fn test_cron_jobs_no_service() {
    let state = test_state();
    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/cron/jobs")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["jobs"].as_array().unwrap().is_empty());
    assert_eq!(json["count"], 0);
    assert_eq!(json["service_available"], false);
}

#[tokio::test]
async fn test_cron_jobs_with_service() {
    let state = test_state();
    let tmp = tempfile::TempDir::new().unwrap();
    let svc = crate::cron::CronService::new(tmp.path().to_path_buf());
    let _ = tokio::fs::create_dir_all(tmp.path().join("cron").join("jobs")).await;
    svc.add(garyx_models::config::CronJobConfig {
        id: "test-job".to_owned(),
        kind: Default::default(),
        label: None,
        schedule: garyx_models::config::CronSchedule::Interval { interval_secs: 60 },
        ui_schedule: None,
        action: garyx_models::config::CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
    })
    .await
    .unwrap();

    // Replace state with cron service
    let mut state_with_cron = (*state).clone_for_test();
    state_with_cron.ops.cron_service = Some(Arc::new(svc));
    let state_with_cron = Arc::new(state_with_cron);

    let router = api_router(state_with_cron);

    let req = Request::builder()
        .uri("/api/cron/jobs")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["count"], 1);
    assert_eq!(json["service_available"], true);
    assert_eq!(json["jobs"][0]["id"], "test-job");
}

#[tokio::test]
async fn test_cron_runs_no_service() {
    let state = test_state();
    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/cron/runs")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["runs"].as_array().unwrap().is_empty());
    assert_eq!(json["count"], 0);
}

#[tokio::test]
async fn test_heartbeat_summary_no_service() {
    let state = test_state();
    let router = api_router(state);

    let req = Request::builder()
        .uri("/api/heartbeat/summary")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["enabled"], true);
    assert_eq!(json["recent_count"], 0);
    assert_eq!(json["service_available"], false);
}

#[tokio::test]
async fn test_heartbeat_summary_with_service() {
    let config = garyx_models::config::HeartbeatConfig::default();
    let svc = Arc::new(crate::heartbeat::HeartbeatService::new(config));
    svc.trigger().await;

    let state = test_state();
    let mut state_with_hb = (*state).clone_for_test();
    state_with_hb.ops.heartbeat_service = Some(svc);
    let state_with_hb = Arc::new(state_with_hb);

    let router = api_router(state_with_hb);

    let req = Request::builder()
        .uri("/api/heartbeat/summary")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["service_available"], true);
    assert_eq!(json["recent_count"], 1);
    assert_eq!(json["successful"], 1);
}

#[tokio::test]
async fn test_heartbeat_trigger() {
    let state = test_state();
    let router = api_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/heartbeat/trigger")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
}

#[tokio::test]
async fn test_settings_update_valid() {
    let state = test_state();
    let router = api_router(state);

    let config = GaryxConfig::default();
    let body_val = serde_json::to_value(&config).unwrap();

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
}

#[tokio::test]
async fn test_settings_update_invalid_json_value() {
    let state = test_state();
    let router = api_router(state);

    // A JSON array instead of an object
    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(b"[1,2,3]".to_vec()))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], false);
}

#[tokio::test]
async fn test_settings_update_rejects_unknown_top_level_field() {
    let state = test_state();
    let router = api_router(state);

    let mut body_val = serde_json::to_value(GaryxConfig::default()).unwrap();
    body_val["unknown_top_level"] = json!(123);

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], false);
    let errors = json["errors"].as_array().unwrap();
    assert!(
        errors
            .iter()
            .any(|e| e.as_str().unwrap_or("").contains("$.unknown_top_level"))
    );
}

#[tokio::test]
async fn test_settings_update_rejects_unknown_nested_field() {
    let state = test_state();
    let router = api_router(state);

    let mut body_val = serde_json::to_value(GaryxConfig::default()).unwrap();
    body_val["gateway"]["unknown_nested"] = json!(true);

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let errors = json["errors"].as_array().unwrap();
    assert!(errors.iter().any(|e| {
        e.as_str()
            .unwrap_or("")
            .contains("$.gateway.unknown_nested")
    }));
}

#[tokio::test]
async fn test_settings_roundtrip_persistence() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    // Write initial config
    let initial = GaryxConfig::default();
    tokio::fs::write(&config_path, serde_json::to_vec_pretty(&initial).unwrap())
        .await
        .unwrap();

    let state = test_state();
    let mut state_with_path = (*state).clone_for_test();
    state_with_path.ops.config_path = Some(config_path.clone());
    let state_with_path = Arc::new(state_with_path);

    let router = api_router(state_with_path.clone());

    // PUT new config with changed port
    let mut new_config = initial.clone();
    new_config.gateway.port = 9999;
    let body_val = serde_json::to_value(&new_config).unwrap();

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    // Verify file was written
    let file_content = tokio::fs::read_to_string(&config_path).await.unwrap();
    eprintln!("persisted_partial_settings={file_content}");
    let persisted: GaryxConfig = serde_json::from_str(&file_content).unwrap();
    assert_eq!(persisted.gateway.port, 9999);

    // Verify live_config was updated
    let live = state_with_path.config_snapshot();
    assert_eq!(live.gateway.port, 9999);
}

#[tokio::test]
async fn test_settings_update_merge_false_deletes_channel_account() {
    // Confirms the `merge=false` PUT flow actually DELETES an account
    // that's been omitted from the body — the default `merge=true` path
    // preserves absent fields by design, so a dedicated full-replace
    // opt-in is the only way a UI can remove an account via the HTTP
    // surface. Exercised against a generic plugin-owned channel
    // (`config.channels.plugins[id].accounts`) so the test stays
    // decoupled from any built-in channel's specific account shape.
    use garyx_models::config::{PluginAccountEntry, PluginChannelConfig};

    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    let mut initial = GaryxConfig::default();
    let mut plugin_cfg = PluginChannelConfig::default();
    plugin_cfg.accounts.insert(
        "bot-to-delete".to_owned(),
        PluginAccountEntry {
            enabled: true,
            name: Some("doomed".to_owned()),
            agent_id: Some("claude".to_owned()),
            workspace_dir: None,
            config: serde_json::json!({ "token": "secret" }),
        },
    );
    initial
        .channels
        .plugins
        .insert("sample_plugin".to_owned(), plugin_cfg);
    tokio::fs::write(&config_path, serde_json::to_vec_pretty(&initial).unwrap())
        .await
        .unwrap();

    let state = test_state();
    let mut state_with_path = (*state).clone_for_test();
    state_with_path.ops.config_path = Some(config_path.clone());
    state_with_path
        .apply_runtime_config(initial.clone())
        .await
        .unwrap();
    let state_with_path = Arc::new(state_with_path);
    let router = api_router(state_with_path.clone());

    // Build a PUT body where the plugin's accounts map is empty.
    let mut without_account = initial.clone();
    without_account
        .channels
        .plugins
        .get_mut("sample_plugin")
        .unwrap()
        .accounts
        .clear();
    let body_val = serde_json::to_value(&without_account).unwrap();

    // Default merge=true: deep-merge preserves the account — confirms
    // the pre-existing safeguard still holds.
    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert!(
        state_with_path
            .config_snapshot()
            .channels
            .plugins
            .get("sample_plugin")
            .map(|cfg| cfg.accounts.contains_key("bot-to-delete"))
            .unwrap_or(false),
        "default merge=true must preserve the account"
    );

    // merge=false: the caller asserts a full-document replace, so the
    // account should actually be gone.
    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings?merge=false")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert!(
        !state_with_path
            .config_snapshot()
            .channels
            .plugins
            .get("sample_plugin")
            .map(|cfg| cfg.accounts.contains_key("bot-to-delete"))
            .unwrap_or(false),
        "merge=false must let the client delete the account"
    );

    let file_content = tokio::fs::read_to_string(&config_path).await.unwrap();
    let persisted: GaryxConfig = serde_json::from_str(&file_content).unwrap();
    assert!(
        !persisted
            .channels
            .plugins
            .get("sample_plugin")
            .map(|cfg| cfg.accounts.contains_key("bot-to-delete"))
            .unwrap_or(false),
        "deletion must be persisted to disk, not just runtime"
    );
}

#[tokio::test]
async fn test_settings_reload_applies_config_from_disk() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    let mut initial = GaryxConfig::default();
    initial.gateway.port = 31337;
    tokio::fs::write(&config_path, serde_json::to_vec_pretty(&initial).unwrap())
        .await
        .unwrap();

    let state = test_state();
    let mut state_with_path = (*state).clone_for_test();
    state_with_path.ops.config_path = Some(config_path.clone());
    state_with_path
        .apply_runtime_config(initial.clone())
        .await
        .unwrap();
    let state_with_path = Arc::new(state_with_path);
    let router = api_router(state_with_path.clone());

    let mut updated = initial.clone();
    updated.gateway.port = 42424;
    tokio::fs::write(&config_path, serde_json::to_vec_pretty(&updated).unwrap())
        .await
        .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/api/settings/reload")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["message"], "config reloaded");
    assert_eq!(state_with_path.config_snapshot().gateway.port, 42424);
}

#[tokio::test]
async fn test_settings_update_partial_payload_preserves_existing_sections() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");

    let mut initial = GaryxConfig::default();
    initial.gateway.port = 4242;
    initial.gateway.image_gen.api_key = "image-secret".to_owned();
    initial
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "telegram-secret".to_owned(),
                    enabled: true,
                    name: None,
                    agent_id: "claude".to_owned(),
                    workspace_dir: None,
                    owner_target: None,
                    groups: std::collections::HashMap::new(),
                },
            ),
        );

    tokio::fs::write(&config_path, serde_json::to_vec_pretty(&initial).unwrap())
        .await
        .unwrap();

    let state = test_state();
    let mut state_with_path = (*state).clone_for_test();
    state_with_path.ops.config_path = Some(config_path.clone());
    state_with_path
        .apply_runtime_config(initial.clone())
        .await
        .unwrap();
    let state_with_path = Arc::new(state_with_path);
    let router = api_router(state_with_path.clone());

    let partial_update = json!({
        "commands": [],
    });

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&partial_update).unwrap()))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let file_content = tokio::fs::read_to_string(&config_path).await.unwrap();
    let persisted: GaryxConfig = serde_json::from_str(&file_content).unwrap();
    assert_eq!(persisted.commands.len(), 0);
    assert_eq!(persisted.gateway.port, 4242);
    assert_eq!(persisted.gateway.image_gen.api_key, "image-secret");
    let telegram = persisted
        .channels
        .plugins
        .get("telegram")
        .and_then(|channel| channel.accounts.get("main"))
        .unwrap();
    assert_eq!(telegram.config["token"], "telegram-secret");
}

#[tokio::test]
async fn test_settings_update_strips_legacy_agent_defaults_workspace_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_path = tmp.path().join("gary.json");
    tokio::fs::write(
        &config_path,
        serde_json::to_vec_pretty(&GaryxConfig::default()).unwrap(),
    )
    .await
    .unwrap();

    let state = test_state();
    let mut state_with_path = (*state).clone_for_test();
    state_with_path.ops.config_path = Some(config_path.clone());
    let state_with_path = Arc::new(state_with_path);
    let router = api_router(state_with_path.clone());

    let mut body_val = serde_json::to_value(GaryxConfig::default()).unwrap();
    body_val["agent_defaults"]["workspace_dir"] = json!("~/gary");
    body_val["gateway"]["port"] = json!(4242);

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let persisted =
        serde_json::from_str::<Value>(&tokio::fs::read_to_string(&config_path).await.unwrap())
            .unwrap();
    assert_eq!(persisted["gateway"]["port"], 4242);
    assert!(persisted["agent_defaults"].get("workspace_dir").is_none());

    let live = serde_json::to_value(state_with_path.config_snapshot()).unwrap();
    assert!(live["agent_defaults"].get("workspace_dir").is_none());
}

#[tokio::test]
async fn test_restart_ok() {
    let state = test_state();
    let router = api_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/restart")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
}

#[tokio::test]
async fn test_restart_cooldown() {
    let state = test_state();

    // Simulate a recent restart
    {
        let mut tracker = state.ops.restart_tracker.lock().await;
        tracker.last_restart = Some(Instant::now());
    }

    let router = api_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/restart")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 429);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["reason"], "cooldown");
}

#[tokio::test]
async fn test_restart_auth_required_no_token() {
    let state = test_state();
    // Create state with auth tokens configured
    let mut state_with_auth = (*state).clone_for_test();
    state_with_auth.ops.restart_tokens = vec!["secret-token-123".to_owned()];
    let state_with_auth = Arc::new(state_with_auth);

    let router = api_router(state_with_auth);

    let req = Request::builder()
        .method("POST")
        .uri("/api/restart")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 403);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["reason"], "unauthorized");
}

#[tokio::test]
async fn test_restart_unauthorized_contract_shape() {
    let state = test_state();
    let mut state_with_auth = (*state).clone_for_test();
    state_with_auth.ops.restart_tokens = vec!["secret-token-123".to_owned()];
    let state_with_auth = Arc::new(state_with_auth);

    let router = api_router(state_with_auth);
    let req = Request::builder()
        .method("POST")
        .uri("/api/restart")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 403);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let got: Value = serde_json::from_slice(&body).unwrap();
    let expected = json!({
        "ok": false,
        "reason": "unauthorized",
        "message": "valid authorization token required for restart",
    });
    assert_eq!(got, expected);
}

#[tokio::test]
async fn test_settings_unknown_field_contract_shape() {
    let state = test_state();
    let router = api_router(state);

    let mut body_val = serde_json::to_value(GaryxConfig::default()).unwrap();
    body_val["unknown_top_level"] = json!(1);

    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body_val).unwrap()))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 400);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let got: Value = serde_json::from_slice(&body).unwrap();
    let expected = json!({
        "ok": false,
        "errors": ["unknown field: $.unknown_top_level"],
    });
    assert_eq!(got, expected);
}

#[tokio::test]
async fn test_restart_auth_required_wrong_token() {
    let state = test_state();
    let mut state_with_auth = (*state).clone_for_test();
    state_with_auth.ops.restart_tokens = vec!["secret-token-123".to_owned()];
    let state_with_auth = Arc::new(state_with_auth);

    let router = api_router(state_with_auth);

    let req = Request::builder()
        .method("POST")
        .uri("/api/restart")
        .header("authorization", "Bearer wrong-token")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 403);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["reason"], "unauthorized");
}

#[tokio::test]
async fn test_restart_auth_required_valid_token() {
    let state = test_state();
    let mut state_with_auth = (*state).clone_for_test();
    state_with_auth.ops.restart_tokens = vec!["secret-token-123".to_owned()];
    let state_with_auth = Arc::new(state_with_auth);

    let router = api_router(state_with_auth);

    let req = Request::builder()
        .method("POST")
        .uri("/api/restart")
        .header("authorization", "Bearer secret-token-123")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
}

#[tokio::test]
async fn test_restart_no_auth_when_tokens_empty() {
    // No restart tokens configured = restart endpoint auth is not required.
    let state = test_state();
    let router = api_router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/api/restart")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_thread_history_detail_with_thread_id() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "thread::u3",
        json!({
            "messages": [
                { "role": "user", "content": "hello" }
            ]
        }),
    )
    .await;

    let router = api_router(state);
    let req = Request::builder()
        .uri("/api/threads/history?thread_id=thread%3A%3Au3&limit=10")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["thread"]["thread_id"], "thread::u3");
    assert_eq!(json["thread"]["thread_key"], "thread::u3");
    assert_eq!(json["session"]["thread_id"], "thread::u3");
    assert_eq!(json["session"]["thread_key"], "thread::u3");
    assert_eq!(json["message_stats"]["total_messages_in_thread"], 1);
    assert_eq!(json["message_stats"]["total_messages_in_session"], 1);
}

#[tokio::test]
async fn test_thread_history_detail_exposes_internal_loop_markers() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "thread::loop-view",
        json!({
            "messages": [
                {
                    "role": "user",
                    "content": "The user wants you to continue working.",
                    "timestamp": "2026-03-15T10:00:00Z",
                    "internal": true,
                    "internal_kind": "loop_continuation",
                    "loop_origin": "auto_continue"
                },
                {
                    "role": "assistant",
                    "content": "当前没有剩余代码任务。",
                    "timestamp": "2026-03-15T10:00:02Z",
                    "internal": true,
                    "internal_kind": "loop_continuation",
                    "loop_origin": "auto_continue"
                }
            ]
        }),
    )
    .await;

    let router = api_router(state);
    let req = Request::builder()
        .uri("/api/threads/history?thread_id=thread%3A%3Aloop-view&limit=10")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["messages"][0]["internal"], true);
    assert_eq!(json["messages"][0]["internal_kind"], "loop_continuation");
    assert_eq!(json["messages"][0]["loop_origin"], "auto_continue");
    assert_eq!(json["messages"][1]["internal"], true);
    assert_eq!(json["messages"][1]["internal_kind"], "loop_continuation");
    assert_eq!(json["messages"][1]["loop_origin"], "auto_continue");
}

// ---------------------------------------------------------------------
// Team block in thread-history response.
// Emitted as a top-level sibling of `thread`; `null` for standalone
// agent threads.
// ---------------------------------------------------------------------

async fn seed_history_team(state: &Arc<AppState>) {
    use crate::agent_teams::UpsertAgentTeamRequest;
    state
        .ops
        .agent_teams
        .upsert_team(UpsertAgentTeamRequest {
            team_id: "product-ship".to_owned(),
            display_name: "Product Ship".to_owned(),
            leader_agent_id: "planner".to_owned(),
            member_agent_ids: vec![
                "planner".to_owned(),
                "coder".to_owned(),
                "reviewer".to_owned(),
            ],
            workflow_text: "Ship the product.".to_owned(),
        })
        .await
        .expect("team upsert");
}

#[tokio::test]
async fn thread_history_emits_null_team_for_standalone_agent_thread() {
    let state = test_state();
    seed_transcript_backed_thread(
        &state,
        "thread::history-standalone",
        json!({
            "agent_id": "claude",
            "provider_type": "claude_code",
            "messages": [
                {"role": "user", "content": "hi", "timestamp": "2026-03-01T00:00:00Z"}
            ]
        }),
    )
    .await;

    let payload = thread_history_for_key(&state, "thread::history-standalone", 10, true).await;
    assert_eq!(payload["ok"], true);
    assert_eq!(
        payload["team"],
        Value::Null,
        "standalone-agent thread must emit team == null"
    );
}

#[tokio::test]
async fn thread_history_emits_team_with_empty_child_map_when_group_missing() {
    let state = test_state();
    seed_history_team(&state).await;
    seed_transcript_backed_thread(
        &state,
        "thread::history-team-fresh",
        json!({
            "agent_id": "product-ship",
            "provider_type": "agent_team",
            "messages": [
                {"role": "user", "content": "kickoff", "timestamp": "2026-03-01T00:00:00Z"}
            ]
        }),
    )
    .await;

    let payload = thread_history_for_key(&state, "thread::history-team-fresh", 10, true).await;
    assert_eq!(payload["ok"], true);
    let team = &payload["team"];
    assert_eq!(team["team_id"], "product-ship");
    assert_eq!(team["display_name"], "Product Ship");
    assert_eq!(team["leader_agent_id"], "planner");
    let child_map = team["child_thread_ids"]
        .as_object()
        .expect("child_thread_ids must be an object, not null");
    assert!(child_map.is_empty());
}

#[tokio::test]
async fn thread_history_projects_known_child_thread_ids_from_group_store() {
    use garyx_bridge::providers::agent_team::Group;
    let state = test_state();
    seed_history_team(&state).await;
    seed_transcript_backed_thread(
        &state,
        "thread::history-team-partial",
        json!({
            "agent_id": "product-ship",
            "provider_type": "agent_team",
            "messages": [
                {"role": "user", "content": "kickoff", "timestamp": "2026-03-01T00:00:00Z"}
            ]
        }),
    )
    .await;

    let mut group = Group::new("thread::history-team-partial", "product-ship");
    group.record_child_thread("coder", "th::child-coder-0001");
    state.ops.agent_team_group_store.save(&group).await;

    let payload = thread_history_for_key(&state, "thread::history-team-partial", 10, true).await;
    assert_eq!(payload["ok"], true);
    let child_map = payload["team"]["child_thread_ids"]
        .as_object()
        .expect("child_thread_ids object");
    assert_eq!(
        child_map.get("coder").and_then(Value::as_str),
        Some("th::child-coder-0001")
    );
    assert!(!child_map.contains_key("reviewer"));
}

#[test]
fn enrich_message_content_for_history_inlines_image_path_blocks() {
    let temp = tempdir().expect("tempdir");
    let image_path = temp.path().join("probe.png");
    std::fs::write(&image_path, b"png-bytes").expect("write image");

    let enriched = enrich_message_content_for_history(&json!([{
        "type": "image",
        "path": image_path.to_string_lossy().to_string(),
        "name": "probe.png",
        "media_type": "image/png"
    }]));
    let blocks = enriched.as_array().expect("array content");
    let source = blocks[0]
        .get("source")
        .and_then(Value::as_object)
        .expect("inline image source");
    assert_eq!(source.get("type").and_then(Value::as_str), Some("base64"));
    assert_eq!(
        source.get("media_type").and_then(Value::as_str),
        Some("image/png")
    );
    assert!(
        source
            .get("data")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    );
}

#[test]
fn humanize_structured_content_mentions_file_blocks() {
    let summary = humanize_structured_content(&json!([
        {
            "type": "text",
            "text": "Please inspect the attachment."
        },
        {
            "type": "file",
            "path": "/tmp/report.pdf",
            "name": "report.pdf"
        }
    ]))
    .expect("summary");
    assert!(summary.contains("Please inspect the attachment."));
    assert!(summary.contains("[File] report.pdf"));
}
