use super::*;
use crate::agent_teams::AgentTeamStore;
use crate::custom_agents::CustomAgentStore;
use crate::garyx_db::{
    CURRENT_TASK_PROJECTION_VERSION, GaryxDbService, RecentThreadDraft, TASK_PROJECTION_NAME,
    TaskProjectionDraft,
};
use crate::server::AppStateBuilder;
use garyx_models::ProviderType;
use garyx_models::config::GaryxConfig;
use std::fs;
use tempfile::tempdir;

fn route_task_source(thread_id: &str, task_id: &str) -> TaskSource {
    TaskSource {
        thread_id: Some(thread_id.to_owned()),
        task_id: Some(task_id.to_owned()),
        task_thread_id: Some(thread_id.to_owned()),
        bot_id: None,
        channel: None,
        account_id: None,
    }
}

fn route_chat_source(thread_id: &str) -> TaskSource {
    TaskSource {
        thread_id: Some(thread_id.to_owned()),
        task_id: None,
        task_thread_id: None,
        bot_id: None,
        channel: None,
        account_id: None,
    }
}

fn route_task_projection_draft(
    thread_id: &str,
    number: u64,
    status: TaskStatus,
    updated_at: &str,
    source: Option<TaskSource>,
) -> TaskProjectionDraft {
    let creator = Principal::Agent {
        agent_id: "test-agent".to_owned(),
    };
    let assignee = Principal::Agent {
        agent_id: "reviewer".to_owned(),
    };
    let updated_by = creator.clone();
    let parent_task_number = source
        .as_ref()
        .and_then(|source| source.task_id.as_deref())
        .and_then(|task_id| task_id.strip_prefix("#TASK-"))
        .and_then(|number| number.parse::<u64>().ok());
    TaskProjectionDraft {
        thread_id: thread_id.to_owned(),
        number,
        status: status.as_str().to_owned(),
        title: format!("Route task {number}"),
        creator_json: serde_json::to_string(&creator).expect("creator json"),
        creator_id: creator.id().to_owned(),
        assignee_json: Some(serde_json::to_string(&assignee).expect("assignee json")),
        assignee_id: Some(assignee.id().to_owned()),
        updated_by_json: serde_json::to_string(&updated_by).expect("updated_by json"),
        executor_json: None,
        source_json: source
            .as_ref()
            .map(|source| serde_json::to_string(source).expect("source json")),
        source_thread_id: source.as_ref().and_then(|source| source.thread_id.clone()),
        source_task_thread_id: source
            .as_ref()
            .and_then(|source| source.task_thread_id.clone()),
        source_task_id: source.as_ref().and_then(|source| source.task_id.clone()),
        parent_task_number,
        source_bot_id: None,
        notification_thread_id: None,
        created_at: "2026-01-01T00:00:00.000Z".to_owned(),
        updated_at: updated_at.to_owned(),
        source_updated_at: updated_at.to_owned(),
        source_events_len: 1,
    }
}

async fn state_with_agent_default_workspace() -> Arc<AppState> {
    let custom_agents = Arc::new(CustomAgentStore::new());
    custom_agents
        .upsert_agent(crate::custom_agents::UpsertCustomAgentRequest {
            agent_id: "reviewer".to_owned(),
            display_name: "Reviewer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            model: Some("gpt-5".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            auth_source: None,
            base_url: None,
            codex_home: None,
            max_tool_iterations: None,
            request_timeout_seconds: None,
            default_workspace_dir: Some("/tmp/agent-task-default".to_owned()),
            avatar_data_url: None,
            system_prompt: "Review carefully.".to_owned(),
        })
        .await
        .expect("custom agent");
    AppStateBuilder::new(GaryxConfig::default())
        .with_custom_agent_store(custom_agents)
        .with_agent_team_store(Arc::new(AgentTeamStore::new()))
        .build()
}

async fn state_with_task_executors() -> Arc<AppState> {
    let mut config = GaryxConfig::default();
    config.tasks.enabled = true;
    let custom_agents = Arc::new(CustomAgentStore::new());
    for agent_id in ["reviewer", "planner", "coder"] {
        custom_agents
            .upsert_agent(crate::custom_agents::UpsertCustomAgentRequest {
                agent_id: agent_id.to_owned(),
                display_name: agent_id.to_owned(),
                provider_type: ProviderType::CodexAppServer,
                model: Some("gpt-5".to_owned()),
                model_reasoning_effort: Some(String::new()),
                model_service_tier: Some(String::new()),
                provider_env: None,
                auth_source: None,
                base_url: None,
                codex_home: None,
                max_tool_iterations: None,
                request_timeout_seconds: None,
                default_workspace_dir: None,
                avatar_data_url: None,
                system_prompt: "Run the task.".to_owned(),
            })
            .await
            .expect("custom agent");
    }
    let agent_teams = Arc::new(AgentTeamStore::new());
    agent_teams
        .upsert_team(crate::agent_teams::UpsertAgentTeamRequest {
            team_id: "product-ship".to_owned(),
            display_name: "Product Ship".to_owned(),
            leader_agent_id: "planner".to_owned(),
            member_agent_ids: vec!["planner".to_owned(), "coder".to_owned()],
            workflow_text: "Coordinate the task.".to_owned(),
            avatar_data_url: None,
        })
        .await
        .expect("agent team");
    AppStateBuilder::new(config)
        .with_custom_agent_store(custom_agents)
        .with_agent_team_store(agent_teams)
        .build()
}

#[tokio::test]
async fn list_task_forest_route_returns_projection_parent_and_run_state() {
    let state = state_with_task_executors().await;
    state
        .ops
        .garyx_db
        .replace_task_projection(route_task_projection_draft(
            "thread::route-parent",
            1,
            TaskStatus::InProgress,
            "2026-01-01T00:00:01.000Z",
            None,
        ))
        .expect("insert parent projection");
    state
        .ops
        .garyx_db
        .replace_task_projection(route_task_projection_draft(
            "thread::route-child",
            2,
            TaskStatus::Todo,
            "2026-01-01T00:00:02.000Z",
            Some(route_task_source("thread::route-parent", "#TASK-1")),
        ))
        .expect("insert child projection");
    state
        .ops
        .garyx_db
        .upsert_recent_thread(RecentThreadDraft {
            thread_id: "thread::route-child".to_owned(),
            title: "Route Child".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: Some("claude_code".to_owned()),
            agent_id: Some("claude".to_owned()),
            message_count: 3,
            last_message_preview: "running".to_owned(),
            recent_run_id: Some("run::route-recent".to_owned()),
            active_run_id: Some("run::route-active".to_owned()),
            run_state: "running".to_owned(),
            updated_at: Some("2026-01-01T00:00:03.000Z".to_owned()),
            last_active_at: "2026-01-01T00:00:04.000Z".to_owned(),
        })
        .expect("insert route recent thread");
    state
        .ops
        .garyx_db
        .record_projection_state(TASK_PROJECTION_NAME, CURRENT_TASK_PROJECTION_VERSION, 2)
        .expect("mark projection current");

    let (status, Json(payload)) = list_task_forest(
        State(state),
        Query(TaskListQuery {
            status: None,
            assignee: None,
            creator: None,
            source_thread_id: None,
            source_task_id: None,
            source_bot_id: None,
            anchor_thread_id: None,
            include_done: true,
            scope: TaskForestScope::All,
            limit: None,
            offset: None,
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["total"], 2);
    assert_eq!(payload["projection_current"], true);
    assert_eq!(
        payload["root_thread_ids"].as_array().expect("root ids"),
        &Vec::<Value>::new()
    );
    assert_eq!(
        payload["skipped_pinned_thread_ids"]
            .as_array()
            .expect("skipped ids"),
        &Vec::<Value>::new()
    );
    let tasks = payload["tasks"].as_array().expect("tasks array");
    let child = tasks
        .iter()
        .find(|task| task["thread_id"] == "thread::route-child")
        .expect("child task");
    assert_eq!(child["parent_task_number"], 1);
    assert_eq!(child["parent_thread_id"], "thread::route-parent");
    assert_eq!(child["active_run_id"], "run::route-active");
    assert_eq!(child["run_state"], "running");
    assert_eq!(child["last_active_at"], "2026-01-01T00:00:04.000Z");
}

#[tokio::test]
async fn list_task_forest_route_defaults_to_pinned_roots_with_metadata() {
    let state = state_with_task_executors().await;
    state
        .ops
        .garyx_db
        .replace_task_projection(route_task_projection_draft(
            "thread::route-child",
            2,
            TaskStatus::Todo,
            "2026-01-01T00:00:02.000Z",
            Some(route_chat_source("thread::route-chat-root")),
        ))
        .expect("insert child projection");
    state
        .ops
        .garyx_db
        .replace_task_projection(route_task_projection_draft(
            "thread::route-grandchild",
            4,
            TaskStatus::InProgress,
            "2026-01-01T00:00:04.000Z",
            Some(route_task_source("thread::route-child", "#TASK-2")),
        ))
        .expect("insert grandchild projection");
    state
        .ops
        .garyx_db
        .replace_task_projection(route_task_projection_draft(
            "thread::route-unrelated",
            3,
            TaskStatus::Todo,
            "2026-01-01T00:00:03.000Z",
            None,
        ))
        .expect("insert unrelated projection");
    state
        .ops
        .garyx_db
        .upsert_recent_thread(RecentThreadDraft {
            thread_id: "thread::route-chat-root".to_owned(),
            title: "Route Chat Root".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: Some("codex".to_owned()),
            agent_id: Some("codex".to_owned()),
            message_count: 5,
            last_message_preview: "Make the forest rooted here".to_owned(),
            recent_run_id: None,
            active_run_id: None,
            run_state: "idle".to_owned(),
            updated_at: Some("2026-01-01T00:00:01.000Z".to_owned()),
            last_active_at: "2026-01-01T00:00:01.000Z".to_owned(),
        })
        .expect("insert root recent thread");
    state
        .ops
        .garyx_db
        .pin_thread("thread::route-chat-root")
        .expect("pin root");
    state
        .ops
        .garyx_db
        .pin_thread("thread::route-chat")
        .expect("pin chat");
    state
        .ops
        .garyx_db
        .record_projection_state(TASK_PROJECTION_NAME, CURRENT_TASK_PROJECTION_VERSION, 3)
        .expect("mark projection current");

    let (status, Json(payload)) = list_task_forest(
        State(state),
        Query(TaskListQuery {
            status: None,
            assignee: None,
            creator: None,
            source_thread_id: None,
            source_task_id: None,
            source_bot_id: None,
            anchor_thread_id: None,
            include_done: true,
            scope: TaskForestScope::default(),
            limit: None,
            offset: None,
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["total"], 2);
    assert_eq!(payload["projection_current"], true);
    assert_eq!(
        payload["root_thread_ids"],
        serde_json::json!(["thread::route-chat-root"])
    );
    assert_eq!(
        payload["skipped_pinned_thread_ids"],
        serde_json::json!(["thread::route-chat"])
    );
    let tasks = payload["tasks"].as_array().expect("tasks array");
    assert_eq!(
        tasks
            .iter()
            .map(|task| task["thread_id"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["thread::route-chat-root", "thread::route-grandchild"]
    );
    assert_eq!(tasks[0]["kind"], "thread");
    assert_eq!(tasks[0]["title"], "Route Chat Root");
    assert_eq!(tasks[1]["kind"], "task");
    assert_eq!(tasks[1]["parent_task_number"], Value::Null);
    assert_eq!(tasks[1]["parent_thread_id"], "thread::route-chat-root");
    assert_eq!(
        tasks[1]["parent_node_id"],
        "thread-root:thread::route-chat-root"
    );
}

#[tokio::test]
async fn list_task_forest_route_supports_anchor_thread_id() {
    let state = state_with_task_executors().await;
    state
        .ops
        .garyx_db
        .replace_task_projection(route_task_projection_draft(
            "thread::route-root",
            10,
            TaskStatus::Done,
            "2026-01-01T00:00:01.000Z",
            None,
        ))
        .expect("insert root projection");
    state
        .ops
        .garyx_db
        .replace_task_projection(route_task_projection_draft(
            "thread::route-child",
            11,
            TaskStatus::InReview,
            "2026-01-01T00:00:02.000Z",
            Some(route_task_source("thread::route-root", "#TASK-10")),
        ))
        .expect("insert child projection");
    state
        .ops
        .garyx_db
        .record_projection_state(TASK_PROJECTION_NAME, CURRENT_TASK_PROJECTION_VERSION, 2)
        .expect("mark projection current");

    let (status, Json(payload)) = list_task_forest(
        State(state),
        Query(TaskListQuery {
            status: None,
            assignee: None,
            creator: None,
            source_thread_id: None,
            source_task_id: None,
            source_bot_id: None,
            anchor_thread_id: Some("thread::route-child".to_owned()),
            include_done: false,
            scope: TaskForestScope::default(),
            limit: None,
            offset: None,
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["total"], 2);
    assert_eq!(payload["active_count"], 1);
    assert_eq!(
        payload["root_thread_ids"],
        serde_json::json!(["thread::route-root"])
    );
    assert_eq!(payload["skipped_pinned_thread_ids"], serde_json::json!([]));
    let tasks = payload["tasks"].as_array().expect("tasks array");
    assert_eq!(
        tasks
            .iter()
            .map(|task| task["thread_id"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["thread::route-root", "thread::route-child"]
    );
    assert_eq!(tasks[0]["kind"], "task");
    assert_eq!(tasks[0]["status"], "done");
    assert_eq!(tasks[0]["depth"], 0);
    assert_eq!(tasks[1]["parent_node_id"], "task:thread::route-root");
    assert_eq!(tasks[1]["depth"], 1);
}

#[tokio::test]
async fn list_task_forest_route_supports_conversation_anchor_thread_id() {
    let state = state_with_task_executors().await;
    state
        .ops
        .garyx_db
        .upsert_recent_thread(RecentThreadDraft {
            thread_id: "thread::route-origin-chat".to_owned(),
            title: "Route Origin Chat".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: Some("codex".to_owned()),
            agent_id: Some("codex".to_owned()),
            message_count: 6,
            last_message_preview: "Derived route tasks".to_owned(),
            recent_run_id: None,
            active_run_id: None,
            run_state: "idle".to_owned(),
            updated_at: Some("2026-01-01T00:00:01.000Z".to_owned()),
            last_active_at: "2026-01-01T00:00:01.000Z".to_owned(),
        })
        .expect("insert route origin recent thread");
    state
        .ops
        .garyx_db
        .replace_task_projection(route_task_projection_draft(
            "thread::route-derived-root",
            30,
            TaskStatus::InProgress,
            "2026-01-01T00:00:02.000Z",
            Some(route_chat_source("thread::route-origin-chat")),
        ))
        .expect("insert derived root projection");
    state
        .ops
        .garyx_db
        .replace_task_projection(route_task_projection_draft(
            "thread::route-derived-child",
            31,
            TaskStatus::InReview,
            "2026-01-01T00:00:03.000Z",
            Some(route_task_source("thread::route-derived-root", "#TASK-30")),
        ))
        .expect("insert derived child projection");
    state
        .ops
        .garyx_db
        .record_projection_state(TASK_PROJECTION_NAME, CURRENT_TASK_PROJECTION_VERSION, 2)
        .expect("mark projection current");

    let (status, Json(payload)) = list_task_forest(
        State(state),
        Query(TaskListQuery {
            status: Some(TaskStatus::Done),
            assignee: None,
            creator: None,
            source_thread_id: Some("thread::unrelated".to_owned()),
            source_task_id: Some("#TASK-999".to_owned()),
            source_bot_id: Some("api:unrelated".to_owned()),
            anchor_thread_id: Some("thread::route-origin-chat".to_owned()),
            include_done: false,
            scope: TaskForestScope::default(),
            limit: Some(1),
            offset: Some(99),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["total"], 3);
    assert_eq!(payload["projection_current"], true);
    assert_eq!(
        payload["root_thread_ids"],
        serde_json::json!(["thread::route-origin-chat"])
    );
    assert_eq!(payload["skipped_pinned_thread_ids"], serde_json::json!([]));
    let tasks = payload["tasks"].as_array().expect("tasks array");
    assert_eq!(
        tasks
            .iter()
            .map(|task| task["thread_id"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec![
            "thread::route-origin-chat",
            "thread::route-derived-root",
            "thread::route-derived-child"
        ]
    );
    assert_eq!(tasks[0]["kind"], "thread");
    assert_eq!(tasks[0]["title"], "Route Origin Chat");
    assert_eq!(tasks[0]["depth"], 0);
    assert_eq!(tasks[1]["kind"], "task");
    assert_eq!(
        tasks[1]["parent_node_id"],
        "thread-root:thread::route-origin-chat"
    );
    assert_eq!(tasks[1]["parent_thread_id"], "thread::route-origin-chat");
    assert_eq!(tasks[1]["parent_task_number"], Value::Null);
    assert_eq!(tasks[1]["depth"], 0);
    assert_eq!(tasks[2]["kind"], "task");
    assert_eq!(
        tasks[2]["parent_node_id"],
        "task:thread::route-derived-root"
    );
    assert_eq!(tasks[2]["parent_thread_id"], "thread::route-derived-root");
    assert_eq!(tasks[2]["parent_task_number"], 30);
    assert_eq!(tasks[2]["depth"], 1);
    assert_eq!(payload["active_count"], 2);
}

/// Headless API smoke for the origin-rooted forest contract: the same
/// tree (including a done leaf) is returned for the source-conversation
/// anchor and for a deep child task anchor, thread root first.
#[tokio::test]
async fn list_task_forest_route_returns_identical_forest_for_conversation_and_task_anchors() {
    let state = state_with_task_executors().await;
    state
        .ops
        .garyx_db
        .upsert_recent_thread(RecentThreadDraft {
            thread_id: "thread::smoke-chat".to_owned(),
            title: "Smoke Chat".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: Some("codex".to_owned()),
            agent_id: Some("codex".to_owned()),
            message_count: 3,
            last_message_preview: "Smoke tree".to_owned(),
            recent_run_id: None,
            active_run_id: None,
            run_state: "idle".to_owned(),
            updated_at: Some("2026-01-01T00:00:01.000Z".to_owned()),
            last_active_at: "2026-01-01T00:00:01.000Z".to_owned(),
        })
        .expect("insert smoke recent thread");
    state
        .ops
        .garyx_db
        .replace_task_projection(route_task_projection_draft(
            "thread::smoke-root",
            40,
            TaskStatus::InProgress,
            "2026-01-01T00:00:02.000Z",
            Some(route_chat_source("thread::smoke-chat")),
        ))
        .expect("insert smoke root");
    state
        .ops
        .garyx_db
        .replace_task_projection(route_task_projection_draft(
            "thread::smoke-done-leaf",
            41,
            TaskStatus::Done,
            "2026-01-01T00:00:03.000Z",
            Some(route_task_source("thread::smoke-root", "#TASK-40")),
        ))
        .expect("insert smoke done leaf");
    state
        .ops
        .garyx_db
        .replace_task_projection(route_task_projection_draft(
            "thread::smoke-deep-child",
            42,
            TaskStatus::InReview,
            "2026-01-01T00:00:04.000Z",
            Some(route_task_source("thread::smoke-root", "#TASK-40")),
        ))
        .expect("insert smoke deep child");
    state
        .ops
        .garyx_db
        .record_projection_state(TASK_PROJECTION_NAME, CURRENT_TASK_PROJECTION_VERSION, 3)
        .expect("mark projection current");

    let mut payloads = Vec::new();
    for anchor in ["thread::smoke-chat", "thread::smoke-deep-child"] {
        let (status, Json(payload)) = list_task_forest(
            State(state.clone()),
            Query(TaskListQuery {
                status: None,
                assignee: None,
                creator: None,
                source_thread_id: None,
                source_task_id: None,
                source_bot_id: None,
                anchor_thread_id: Some(anchor.to_owned()),
                include_done: false,
                scope: TaskForestScope::default(),
                limit: None,
                offset: None,
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "anchor {anchor}");
        payloads.push(payload);
    }

    for (payload, anchor) in payloads
        .iter()
        .zip(["thread::smoke-chat", "thread::smoke-deep-child"])
    {
        let tasks = payload["tasks"].as_array().expect("tasks array");
        assert_eq!(
            tasks
                .iter()
                .map(|task| task["node_id"].as_str().unwrap_or_default())
                .collect::<Vec<_>>(),
            vec![
                "thread-root:thread::smoke-chat",
                "task:thread::smoke-root",
                "task:thread::smoke-deep-child",
                "task:thread::smoke-done-leaf",
            ],
            "anchor {anchor}"
        );
        assert_eq!(tasks[0]["kind"], "thread", "anchor {anchor}");
        assert_eq!(tasks[0]["title"], "Smoke Chat", "anchor {anchor}");
        assert_eq!(
            tasks[3]["status"], "done",
            "done leaf retained for {anchor}"
        );
        assert_eq!(payload["active_count"], 2, "anchor {anchor}");
        assert_eq!(
            payload["root_thread_ids"],
            serde_json::json!(["thread::smoke-chat"]),
            "anchor {anchor}"
        );
    }
    assert_eq!(
        payloads[0]["tasks"], payloads[1]["tasks"],
        "conversation and deep task anchors must return the identical forest"
    );
}

#[tokio::test]
async fn task_runtime_uses_assignee_default_workspace_when_unset() {
    let state = state_with_agent_default_workspace().await;
    let runtime = task_runtime_with_default_workspace(&state, None, Some("reviewer"))
        .await
        .expect("runtime");

    assert_eq!(
        runtime.and_then(|runtime| runtime.workspace_dir).as_deref(),
        Some("/tmp/agent-task-default")
    );
}

#[tokio::test]
async fn task_runtime_explicit_workspace_overrides_agent_default() {
    let state = state_with_agent_default_workspace().await;
    let runtime = task_runtime_with_default_workspace(
        &state,
        Some(TaskRuntimeInput {
            agent_id: Some("reviewer".to_owned()),
            workspace_dir: Some("/tmp/task-explicit".to_owned()),
            workspace_mode: WorkspaceMode::Local,
            worktree_base_dir: None,
        }),
        Some("reviewer"),
    )
    .await
    .expect("runtime");

    assert_eq!(
        runtime.and_then(|runtime| runtime.workspace_dir).as_deref(),
        Some("/tmp/task-explicit")
    );
}

#[tokio::test]
async fn task_runtime_without_agent_default_keeps_workspace_unset() {
    let state = AppStateBuilder::new(GaryxConfig::default())
        .with_custom_agent_store(Arc::new(CustomAgentStore::new()))
        .with_agent_team_store(Arc::new(AgentTeamStore::new()))
        .build();
    let runtime = task_runtime_with_default_workspace(&state, None, Some("claude"))
        .await
        .expect("runtime");

    assert!(runtime.is_none());
}

#[tokio::test]
async fn agent_executor_creates_in_progress_task_and_dispatches_without_assignee() {
    let state = state_with_task_executors().await;

    let (status, Json(payload)) = create_task(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateTaskBody {
            title: Some("Agent executor".to_owned()),
            body: Some("Implement the slice.".to_owned()),
            assignee: None,
            notification_target: Some(TaskNotificationTargetBody::None),
            source: None,
            executor: Some(TaskExecutorBody::Agent {
                agent_id: "reviewer".to_owned(),
            }),
            start: false,
            actor: None,
            agent_id: None,
            workspace_dir: None,
            runtime: None,
        }),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(payload["task"]["status"], "in_progress");
    assert!(payload["task"]["assignee"].is_null());
    assert_eq!(payload["task"]["executor"]["type"], "agent");
    assert_eq!(payload["task"]["executor"]["agent_id"], "reviewer");
    assert_eq!(payload["dispatch"]["queued"], true);
    assert_eq!(payload["dispatch"]["agent_id"], "reviewer");
    let thread_id = payload["thread_id"].as_str().expect("thread id");
    let stored = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("stored thread");
    assert_eq!(stored["agent_id"], "reviewer");
    assert_eq!(stored["provider_type"], "codex_app_server");
}

#[tokio::test]
async fn team_executor_binds_team_and_rejects_standalone_agent() {
    let state = state_with_task_executors().await;

    let (status, Json(payload)) = create_task(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateTaskBody {
            title: Some("Team executor".to_owned()),
            body: None,
            assignee: None,
            notification_target: Some(TaskNotificationTargetBody::None),
            source: None,
            executor: Some(TaskExecutorBody::Team {
                team_id: "product-ship".to_owned(),
            }),
            start: false,
            actor: None,
            agent_id: None,
            workspace_dir: None,
            runtime: None,
        }),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(payload["task"]["status"], "in_progress");
    assert!(payload["task"]["assignee"].is_null());
    assert_eq!(payload["task"]["executor"]["type"], "team");
    assert_eq!(payload["task"]["executor"]["team_id"], "product-ship");
    assert_eq!(payload["dispatch"]["queued"], true);
    assert_eq!(payload["dispatch"]["agent_id"], "product-ship");
    let thread_id = payload["thread_id"].as_str().expect("thread id");
    let stored = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("stored thread");
    assert_eq!(stored["agent_id"], "product-ship");
    assert_eq!(stored["provider_type"], "agent_team");

    let (status, Json(payload)) = create_task(
        State(state),
        HeaderMap::new(),
        Json(CreateTaskBody {
            title: Some("Bad team executor".to_owned()),
            body: None,
            assignee: None,
            notification_target: Some(TaskNotificationTargetBody::None),
            source: None,
            executor: Some(TaskExecutorBody::Team {
                team_id: "reviewer".to_owned(),
            }),
            start: false,
            actor: None,
            agent_id: None,
            workspace_dir: None,
            runtime: None,
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        payload["error"]
            .as_str()
            .unwrap()
            .contains("requires an agent team")
    );
}

#[tokio::test]
async fn executor_rejects_assignee_and_team_as_agent() {
    let state = state_with_task_executors().await;

    let (status, Json(payload)) = create_task(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateTaskBody {
            title: Some("Mixed executor".to_owned()),
            body: None,
            assignee: Some(Principal::Agent {
                agent_id: "reviewer".to_owned(),
            }),
            notification_target: Some(TaskNotificationTargetBody::None),
            source: None,
            executor: Some(TaskExecutorBody::Agent {
                agent_id: "reviewer".to_owned(),
            }),
            start: false,
            actor: None,
            agent_id: None,
            workspace_dir: None,
            runtime: None,
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        payload["error"]
            .as_str()
            .unwrap()
            .contains("cannot also set an assignee")
    );

    let (status, Json(payload)) = create_task(
        State(state),
        HeaderMap::new(),
        Json(CreateTaskBody {
            title: Some("Bad agent executor".to_owned()),
            body: None,
            assignee: None,
            notification_target: Some(TaskNotificationTargetBody::None),
            source: None,
            executor: Some(TaskExecutorBody::Agent {
                agent_id: "product-ship".to_owned(),
            }),
            start: false,
            actor: None,
            agent_id: None,
            workspace_dir: None,
            runtime: None,
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        payload["error"]
            .as_str()
            .unwrap()
            .contains("requires a standalone agent")
    );
}

#[tokio::test]
async fn workflow_backed_task_creation_dispatches_workflow_entrypoint() {
    let data_dir = tempdir().expect("data dir");
    let mut config = GaryxConfig::default();
    config.tasks.enabled = true;
    config.sessions.data_dir = Some(data_dir.path().join("data").to_string_lossy().to_string());
    let workflow_package = data_dir.path().join("workflows").join("unit");
    fs::create_dir_all(&workflow_package).expect("workflow package");
    fs::write(
        workflow_package.join("garyx.workflow.json"),
        r#"{
          "workflowId": "unit",
          "version": 4,
          "name": "Unit Workflow",
          "input": {"placeholder": "Unit request"},
          "defaults": {}
        }"#,
    )
    .expect("workflow manifest");
    fs::write(workflow_package.join("workflow.ts"), "export {};\n").expect("workflow source");
    let garyx_db = Arc::new(GaryxDbService::memory().expect("memory db"));
    let state = AppStateBuilder::new(config)
        .with_garyx_db(garyx_db)
        .with_custom_agent_store(Arc::new(CustomAgentStore::new()))
        .with_agent_team_store(Arc::new(AgentTeamStore::new()))
        .build();

    let task_workspace_dir = "/Users/test/workflow-task";
    let old_bun = std::env::var_os("GARYX_WORKFLOW_BUN_BIN");
    unsafe {
        std::env::set_var("GARYX_WORKFLOW_BUN_BIN", "/usr/bin/true");
    }
    let (status, Json(payload)) = create_task(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateTaskBody {
            title: Some("Run workflow".to_owned()),
            body: None,
            assignee: None,
            notification_target: Some(TaskNotificationTargetBody::None),
            source: None,
            executor: Some(TaskExecutorBody::Workflow {
                workflow_id: "unit".to_owned(),
                input: Some(json!({"question": "test"})),
            }),
            start: false,
            actor: None,
            agent_id: Some("claude".to_owned()),
            workspace_dir: Some(task_workspace_dir.to_owned()),
            runtime: None,
        }),
    )
    .await;
    unsafe {
        if let Some(value) = old_bun {
            std::env::set_var("GARYX_WORKFLOW_BUN_BIN", value);
        } else {
            std::env::remove_var("GARYX_WORKFLOW_BUN_BIN");
        }
    }

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(payload["dispatch"]["kind"], "workflow_entrypoint");
    assert_eq!(payload["dispatch"]["workflowId"], "unit");
    assert_eq!(payload["dispatch"]["workflowVersion"], 4);
    assert_eq!(payload["task"]["status"], "in_progress");
    assert_eq!(payload["task"]["executor"]["type"], "workflow");
    assert_eq!(payload["task"]["executor"]["workflow_id"], "unit");
    assert_eq!(payload["task"]["executor"]["workflow_version"], 4);
    let task_thread_id = payload["thread_id"].as_str().expect("thread id");
    let thread_record = state
        .threads
        .thread_store
        .get(task_thread_id)
        .await
        .expect("task thread");
    assert_eq!(thread_record["workspace_dir"], task_workspace_dir);
}

#[test]
fn workflow_task_input_defaults_to_task_body() {
    assert_eq!(
        workflow_task_input_or_body(None, Some("  run this workflow  ")),
        json!("run this workflow")
    );
    assert_eq!(
        workflow_task_input_or_body(Some(json!({"explicit": true})), Some("ignored")),
        json!({"explicit": true})
    );
    assert_eq!(workflow_task_input_or_body(None, Some("   ")), Value::Null);
    assert_eq!(workflow_task_input_or_body(None, None), Value::Null);
}
