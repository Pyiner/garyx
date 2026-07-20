use super::{
    AutomationThreadsParams, CreateAutomationBody, UpdateAutomationBody, automation_agent_id,
    automation_threads, build_automation_job, create_automation, is_automation_job,
    list_automations, to_summary, update_automation,
};
use crate::automation::mapping::{compile_schedule, infer_schedule_view, parse_time_hm};
use crate::automation::engine::{CronJob, CronService, JobRunStatus};
use crate::garyx_db::AutomationThreadRunDraft;
use crate::server::AppStateBuilder;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::Utc;
use garyx_models::config::AutomationScheduleView;
use garyx_models::config::{CronAction, CronJobConfig, CronJobKind, CronSchedule, GaryxConfig};
use std::collections::BTreeSet;
use std::sync::Arc;

#[test]
fn parse_time_requires_two_digits() {
    assert_eq!(parse_time_hm("09:05"), Ok((9, 5)));
    assert!(parse_time_hm("9:05").is_err());
    assert!(parse_time_hm("09:5").is_err());
    assert!(parse_time_hm("9:5").is_err());
}

#[test]
fn compile_schedule_daily_rejects_non_hhmm_time() {
    let schedule = AutomationScheduleView::Daily {
        time: "9:5".to_owned(),
        weekdays: vec!["mon".to_owned()],
        timezone: "Asia/Shanghai".to_owned(),
    };
    assert!(compile_schedule(&schedule).is_err());
}

#[test]
fn compile_schedule_interval_rejects_overflow_hours() {
    let schedule = AutomationScheduleView::Interval {
        hours: ((i64::MAX as u64) / 3600) + 1,
    };
    assert!(compile_schedule(&schedule).is_err());
}

#[test]
fn compile_schedule_monthly_builds_day_of_month_cron() {
    let schedule = AutomationScheduleView::Monthly {
        day: 24,
        time: "08:00".to_owned(),
        timezone: "Asia/Shanghai".to_owned(),
    };
    assert_eq!(
        compile_schedule(&schedule).unwrap(),
        CronSchedule::Cron {
            expr: "0 0 8 24 * *".to_owned(),
            timezone: Some("Asia/Shanghai".to_owned()),
        }
    );
}

#[test]
fn compile_schedule_monthly_rejects_invalid_day() {
    let schedule = AutomationScheduleView::Monthly {
        day: 0,
        time: "08:00".to_owned(),
        timezone: "Asia/Shanghai".to_owned(),
    };
    assert!(compile_schedule(&schedule).is_err());
}

#[test]
fn compile_schedule_once_accepts_datetime_local() {
    let schedule = AutomationScheduleView::Once {
        at: "2030-05-01T08:30".to_owned(),
    };
    assert!(matches!(
        compile_schedule(&schedule).unwrap(),
        CronSchedule::Once { .. }
    ));
}

#[test]
fn infer_schedule_view_from_interval() {
    let schedule = CronSchedule::Interval {
        interval_secs: 6 * 3600,
    };
    assert_eq!(
        infer_schedule_view(&schedule).unwrap(),
        AutomationScheduleView::Interval { hours: 6 }
    );
}

#[test]
fn infer_schedule_view_from_daily_cron() {
    let schedule = CronSchedule::Cron {
        expr: "0 30 9 * * MON-FRI".to_owned(),
        timezone: Some("Asia/Shanghai".to_owned()),
    };
    assert_eq!(
        infer_schedule_view(&schedule).unwrap(),
        AutomationScheduleView::Daily {
            time: "09:30".to_owned(),
            weekdays: vec![
                "mo".to_owned(),
                "tu".to_owned(),
                "we".to_owned(),
                "th".to_owned(),
                "fr".to_owned(),
            ],
            timezone: "Asia/Shanghai".to_owned(),
        }
    );
}

#[test]
fn infer_schedule_view_from_monthly_cron() {
    let schedule = CronSchedule::Cron {
        expr: "0 0 8 24 * *".to_owned(),
        timezone: Some("Asia/Shanghai".to_owned()),
    };
    assert_eq!(
        infer_schedule_view(&schedule).unwrap(),
        AutomationScheduleView::Monthly {
            day: 24,
            time: "08:00".to_owned(),
            timezone: "Asia/Shanghai".to_owned(),
        }
    );
}

#[test]
fn infer_schedule_view_from_once() {
    let schedule = CronSchedule::Once {
        at: "2030-05-01T00:30:00Z".to_owned(),
    };
    assert!(matches!(
        infer_schedule_view(&schedule).unwrap(),
        AutomationScheduleView::Once { .. }
    ));
}

#[test]
fn infer_schedule_view_rejects_subhour_intervals() {
    let schedule = CronSchedule::Interval {
        interval_secs: 1800,
    };
    assert!(infer_schedule_view(&schedule).is_err());
}

#[test]
fn build_automation_job_persists_selected_agent() {
    let cfg = build_automation_job(
        "automation::digest",
        "Digest",
        "Summarize recent updates.",
        Some("codex"),
        Some("/tmp/repo"),
        None,
        AutomationScheduleView::Interval { hours: 6 },
        true,
    )
    .expect("automation job");

    assert_eq!(cfg.agent_id.as_deref(), Some("codex"));
}

#[test]
fn build_automation_job_preserves_target_thread() {
    let cfg = build_automation_job(
        "automation::thread-digest",
        "Thread Digest",
        "Summarize this thread.",
        Some("codex"),
        None,
        Some("thread::target"),
        AutomationScheduleView::Interval { hours: 6 },
        true,
    )
    .expect("automation job");

    assert_eq!(cfg.thread_id.as_deref(), Some("thread::target"));
    assert!(cfg.workspace_dir.is_none());
}

#[test]
fn update_automation_body_decodes_target_thread_tristate() {
    let absent: UpdateAutomationBody =
        serde_json::from_value(serde_json::json!({ "label": "Digest" })).unwrap();
    assert!(absent.target_thread_id.is_none());

    let clear: UpdateAutomationBody =
        serde_json::from_value(serde_json::json!({ "targetThreadId": null })).unwrap();
    assert!(matches!(clear.target_thread_id, Some(None)));

    let set: UpdateAutomationBody =
        serde_json::from_value(serde_json::json!({ "targetThreadId": "thread::target" })).unwrap();
    assert_eq!(
        set.target_thread_id
            .as_ref()
            .and_then(|value| value.as_deref()),
        Some("thread::target")
    );
}

#[test]
fn thread_only_prompt_jobs_are_automation_jobs() {
    let job = CronJob {
        id: "automation::thread-only".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Thread Only".to_owned()),
        schedule: CronSchedule::Interval {
            interval_secs: 3600,
        },
        ui_schedule: Some(AutomationScheduleView::Interval { hours: 1 }),
        action: CronAction::AgentTurn,
        target: None,
        message: Some("Summarize this thread.".to_owned()),
        workspace_dir: None,
        agent_id: Some("codex".to_owned()),
        thread_id: Some("thread::target".to_owned()),
        delete_after_run: false,
        enabled: true,
        next_run: Utc::now(),
        last_status: JobRunStatus::NeverRun,
        run_count: 0,
        created_at: Utc::now(),
        last_run_at: None,
        system: false,
        validation_error: None,
    };

    assert!(is_automation_job(&job));
}

#[tokio::test]
async fn automation_summary_defaults_agent_to_claude_for_legacy_jobs() {
    let job = CronJob {
        id: "automation::legacy".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Legacy".to_owned()),
        schedule: CronSchedule::Interval {
            interval_secs: 3600,
        },
        ui_schedule: Some(AutomationScheduleView::Interval { hours: 1 }),
        action: CronAction::AgentTurn,
        target: None,
        message: Some("Summarize recent updates.".to_owned()),
        workspace_dir: Some("/tmp/repo".to_owned()),
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        next_run: Utc::now(),
        last_status: JobRunStatus::NeverRun,
        run_count: 0,
        created_at: Utc::now(),
        last_run_at: None,
        system: false,
        validation_error: None,
    };

    assert_eq!(automation_agent_id(&job), "claude");
    let state = AppStateBuilder::new(GaryxConfig::default()).build();
    let summary = to_summary(&state, &job, None).await.expect("summary");
    assert_eq!(summary.agent_id.as_deref(), Some("claude"));
    assert_eq!(summary.effective_agent_id.as_deref(), Some("claude"));
}

#[tokio::test]
async fn automation_summary_exposes_bound_target_thread() {
    let job = CronJob {
        id: "automation::bound".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Bound".to_owned()),
        schedule: CronSchedule::Interval {
            interval_secs: 3600,
        },
        ui_schedule: Some(AutomationScheduleView::Interval { hours: 1 }),
        action: CronAction::AgentTurn,
        target: None,
        message: Some("Summarize this thread.".to_owned()),
        workspace_dir: None,
        // Legacy dirty cache: target execution must ignore this value and
        // derive the target thread's live binding instead.
        agent_id: Some("claude".to_owned()),
        thread_id: Some("thread::target".to_owned()),
        delete_after_run: false,
        enabled: true,
        next_run: Utc::now(),
        last_status: JobRunStatus::NeverRun,
        run_count: 0,
        created_at: Utc::now(),
        last_run_at: None,
        system: false,
        validation_error: None,
    };

    let state = AppStateBuilder::new(GaryxConfig::default()).build();
    state
        .threads
        .thread_store
        .set("thread::target", serde_json::json!({ "agent_id": "codex" }))
        .await
        .unwrap();
    let summary = to_summary(&state, &job, None).await.expect("summary");
    assert_eq!(summary.target_thread_id.as_deref(), Some("thread::target"));
    assert_eq!(summary.thread_id.as_deref(), Some("thread::target"));
    assert_eq!(summary.workspace_dir, "");
    assert!(summary.agent_id.is_none());
    assert_eq!(summary.effective_agent_id.as_deref(), Some("codex"));
    assert_eq!(
        summary.agent_resolution,
        super::AutomationAgentResolution::FollowThread
    );
    let wire = serde_json::to_value(&summary).unwrap();
    assert!(wire["agentId"].is_null());
    assert_eq!(wire["effectiveAgentId"], "codex");
}

#[tokio::test]
async fn automation_summary_target_missing_uses_typed_nullable_wire_state() {
    let job = CronJob {
        id: "automation::missing-target".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Missing target".to_owned()),
        schedule: CronSchedule::Interval {
            interval_secs: 3600,
        },
        ui_schedule: Some(AutomationScheduleView::Interval { hours: 1 }),
        action: CronAction::AgentTurn,
        target: None,
        message: Some("Summarize this thread.".to_owned()),
        workspace_dir: None,
        agent_id: Some("claude".to_owned()),
        thread_id: Some("thread::missing-target".to_owned()),
        delete_after_run: false,
        enabled: true,
        next_run: Utc::now(),
        last_status: JobRunStatus::NeverRun,
        run_count: 0,
        created_at: Utc::now(),
        last_run_at: None,
        system: false,
        validation_error: None,
    };
    let state = AppStateBuilder::new(GaryxConfig::default()).build();
    let summary = to_summary(&state, &job, None).await.unwrap();
    assert_eq!(
        summary.agent_resolution,
        super::AutomationAgentResolution::TargetMissing
    );
    let wire = serde_json::to_value(summary).unwrap();
    assert!(wire.get("agentId").is_some_and(serde_json::Value::is_null));
    assert!(
        wire.get("effectiveAgentId")
            .is_some_and(serde_json::Value::is_null)
    );
}

#[tokio::test]
async fn automation_list_keeps_legacy_delivery_target_rows_visible() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(CronService::new(temp.path().to_path_buf()));
    service
        .add(CronJobConfig {
            id: "automation::legacy-last".to_owned(),
            kind: CronJobKind::AutomationPrompt,
            label: Some("Legacy delivery".to_owned()),
            schedule: CronSchedule::Interval {
                interval_secs: 3600,
            },
            ui_schedule: Some(AutomationScheduleView::Interval { hours: 1 }),
            action: CronAction::AgentTurn,
            target: Some("last".to_owned()),
            message: Some("Summarize the last delivery target.".to_owned()),
            workspace_dir: None,
            agent_id: Some("claude".to_owned()),
            thread_id: None,
            delete_after_run: false,
            enabled: true,
            system: false,
        })
        .await
        .unwrap();
    let state = AppStateBuilder::new(GaryxConfig::default())
        .with_cron_service(service)
        .build();

    let response = list_automations(State(state)).await.into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let wire: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(wire["automations"].as_array().unwrap().len(), 1);
    assert_eq!(wire["automations"][0]["workspaceDir"], "");
}

#[tokio::test]
async fn unsupported_legacy_schedule_lists_and_repairs_without_rewriting_raw_schedule() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(CronService::new(temp.path().to_path_buf()));
    service
        .add(CronJobConfig {
            id: "automation::repair-schedule".to_owned(),
            kind: CronJobKind::AutomationPrompt,
            label: Some("Repair me".to_owned()),
            schedule: CronSchedule::Interval { interval_secs: 90 },
            ui_schedule: None,
            action: CronAction::AgentTurn,
            target: None,
            message: Some("Repair this legacy row.".to_owned()),
            workspace_dir: None,
            agent_id: None,
            thread_id: None,
            delete_after_run: false,
            enabled: true,
            system: false,
        })
        .await
        .unwrap();
    let state = AppStateBuilder::new(GaryxConfig::default())
        .with_cron_service(service.clone())
        .build();

    let response = list_automations(State(state.clone())).await.into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let wire: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(wire["automations"][0]["validationState"], "invalid");
    assert_eq!(wire["automations"][0]["schedule"]["kind"], "once");

    let repair: UpdateAutomationBody = serde_json::from_value(serde_json::json!({
        "label": "Repaired",
        "workspaceDir": "/tmp/repaired-automation",
        "agentId": "claude"
    }))
    .unwrap();
    let response = update_automation(
        State(state),
        Path("automation::repair-schedule".to_owned()),
        Json(repair),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let repaired = service.get("automation::repair-schedule").await.unwrap();
    assert_eq!(
        repaired.schedule,
        CronSchedule::Interval { interval_secs: 90 }
    );
    assert!(repaired.ui_schedule.is_none());
    assert!(repaired.validation_error.is_none());
}

#[tokio::test]
async fn update_automation_null_target_thread_clears_binding() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(CronService::new(temp.path().to_path_buf()));
    service
        .add(CronJobConfig {
            id: "automation::bound".to_owned(),
            kind: CronJobKind::AutomationPrompt,
            label: Some("Bound".to_owned()),
            schedule: CronSchedule::Interval {
                interval_secs: 3600,
            },
            ui_schedule: Some(AutomationScheduleView::Interval { hours: 1 }),
            action: CronAction::AgentTurn,
            target: None,
            message: Some("Summarize this thread.".to_owned()),
            workspace_dir: None,
            agent_id: Some("claude".to_owned()),
            thread_id: Some("thread::target".to_owned()),
            delete_after_run: false,
            enabled: true,
            system: false,
        })
        .await
        .unwrap();
    let state = AppStateBuilder::new(GaryxConfig::default())
        .with_cron_service(service.clone())
        .build();
    state
        .threads
        .thread_store
        .set(
            "thread::target",
            serde_json::json!({ "agent_id": "claude" }),
        )
        .await
        .unwrap();
    let body: UpdateAutomationBody = serde_json::from_value(serde_json::json!({
        "targetThreadId": null,
        "workspaceDir": "/tmp/repo"
    }))
    .unwrap();

    let response = update_automation(
        State(state),
        Path("automation::bound".to_owned()),
        Json(body),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    let updated = service
        .get("automation::bound")
        .await
        .expect("automation still exists");
    assert!(updated.thread_id.is_none());
    assert_eq!(updated.workspace_dir.as_deref(), Some("/tmp/repo"));
}

#[tokio::test]
async fn update_automation_switches_target_thread_without_workspace_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(CronService::new(temp.path().to_path_buf()));
    service
        .add(CronJobConfig {
            id: "automation::bound".to_owned(),
            kind: CronJobKind::AutomationPrompt,
            label: Some("Bound".to_owned()),
            schedule: CronSchedule::Interval {
                interval_secs: 3600,
            },
            ui_schedule: Some(AutomationScheduleView::Interval { hours: 1 }),
            action: CronAction::AgentTurn,
            target: None,
            message: Some("Summarize this thread.".to_owned()),
            workspace_dir: Some("/tmp/stale-snapshot".to_owned()),
            agent_id: Some("claude".to_owned()),
            thread_id: Some("thread::target-one".to_owned()),
            delete_after_run: false,
            enabled: true,
            system: false,
        })
        .await
        .unwrap();
    let state = AppStateBuilder::new(GaryxConfig::default())
        .with_cron_service(service.clone())
        .build();
    state
        .threads
        .thread_store
        .set(
            "thread::target-two",
            serde_json::json!({
                "workspace_dir": "/tmp/target-two",
                "metadata": {
                    "agent_id": "claude"
                }
            }),
        )
        .await
        .unwrap();
    let body: UpdateAutomationBody =
        serde_json::from_value(serde_json::json!({ "targetThreadId": "thread::target-two" }))
            .unwrap();

    let response = update_automation(
        State(state),
        Path("automation::bound".to_owned()),
        Json(body),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    let updated = service
        .get("automation::bound")
        .await
        .expect("automation still exists");
    assert_eq!(updated.thread_id.as_deref(), Some("thread::target-two"));
    assert!(updated.workspace_dir.is_none());
}

#[tokio::test]
async fn automation_threads_endpoint_returns_generated_run_associations() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(CronService::new(temp.path().to_path_buf()));
    service
        .add(CronJobConfig {
            id: "automation::daily".to_owned(),
            kind: CronJobKind::AutomationPrompt,
            label: Some("Daily Review".to_owned()),
            schedule: CronSchedule::Interval {
                interval_secs: 3600,
            },
            ui_schedule: Some(AutomationScheduleView::Interval { hours: 1 }),
            action: CronAction::AgentTurn,
            target: None,
            message: Some("Summarize.".to_owned()),
            workspace_dir: Some("/Users/test/project".to_owned()),
            agent_id: Some("codex".to_owned()),
            thread_id: None,
            delete_after_run: false,
            enabled: true,
            system: false,
        })
        .await
        .unwrap();
    let state = AppStateBuilder::new(GaryxConfig::default())
        .with_cron_service(service)
        .build();
    state
        .threads
        .thread_store
        .set(
            "thread::generated",
            serde_json::json!({
                "label": "Daily Review",
                "workspace_dir": "/Users/test/project",
                "agent_id": "codex",
                "automation_id": "automation::daily",
                "automation_thread_mode": "generated_thread",
                "messages": [{"role": "user", "content": "Summarize."}]
            }),
        )
        .await
        .unwrap();
    state
        .ops
        .garyx_db
        .upsert_automation_thread_run(AutomationThreadRunDraft {
            automation_id: "automation::daily".to_owned(),
            run_id: "run-1".to_owned(),
            thread_id: "thread::generated".to_owned(),
            workspace_dir: Some("/Users/test/project".to_owned()),
            agent_id: Some("codex".to_owned()),
            automation_label_snapshot: Some("Daily Review".to_owned()),
            mode: "generated_thread".to_owned(),
            status: "running".to_owned(),
            started_at: "2026-05-28T00:00:00Z".to_owned(),
            finished_at: None,
        })
        .unwrap();

    let response = automation_threads(
        State(state),
        Path("automation::daily".to_owned()),
        Query(AutomationThreadsParams {
            limit: 50,
            offset: 0,
            mode: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["items"][0]["threadId"], "thread::generated");
    assert_eq!(
        payload["items"][0]["thread"]["automationThreadMode"],
        "generated_thread"
    );
    assert_eq!(
        payload["items"][0]["thread"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "activeRunId",
            "agentId",
            "automationId",
            "automationThreadMode",
            "createdAt",
            "id",
            "label",
            "lastAssistantMessage",
            "lastUserMessage",
            "messageCount",
            "providerType",
            "recentRunId",
            "threadId",
            "threadType",
            "title",
            "updatedAt",
            "workspaceDir",
        ]),
        "automation drilldown summary has one exact wire shape"
    );
}

fn target_automation_config(id: &str, thread_id: &str) -> CronJobConfig {
    CronJobConfig {
        id: id.to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Target automation".to_owned()),
        schedule: CronSchedule::Interval {
            interval_secs: 3600,
        },
        ui_schedule: Some(AutomationScheduleView::Interval { hours: 1 }),
        action: CronAction::AgentTurn,
        target: None,
        message: Some("Continue the target thread.".to_owned()),
        workspace_dir: None,
        // Deliberately wrong legacy cache; target mode must ignore it.
        agent_id: Some("claude".to_owned()),
        thread_id: Some(thread_id.to_owned()),
        delete_after_run: false,
        enabled: true,
        system: false,
    }
}

#[tokio::test]
async fn invalid_threadless_automation_stays_visible_and_repairable_with_null_agent_wire() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(CronService::new(temp.path().to_path_buf()));
    let mut invalid = target_automation_config("automation::invalid", "thread::unused");
    invalid.thread_id = None;
    invalid.agent_id = None;
    service.add(invalid).await.unwrap();
    let state = AppStateBuilder::new(GaryxConfig::default())
        .with_cron_service(service)
        .build();

    let response = list_automations(State(state)).await.into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let row = &payload["automations"][0];
    assert_eq!(row["validationState"], "invalid");
    assert_eq!(
        row["validationError"],
        "missing canonical target for agent turn"
    );
    assert!(row.get("agentId").is_some_and(serde_json::Value::is_null));
    assert!(
        row.get("effectiveAgentId")
            .is_some_and(serde_json::Value::is_null)
    );
}

#[tokio::test]
async fn target_mode_ignores_disabled_job_agent_but_conversion_gates_live_thread_binding() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(CronService::new(temp.path().to_path_buf()));
    service
        .add(target_automation_config(
            "automation::target-disabled",
            "thread::target-disabled",
        ))
        .await
        .unwrap();
    let custom_agents = Arc::new(crate::custom_agents::CustomAgentStore::new());
    custom_agents
        .set_enabled("codex", false)
        .await
        .expect("disable target binding");
    let state = AppStateBuilder::new(GaryxConfig::default())
        .with_custom_agent_store(custom_agents)
        .with_cron_service(service.clone())
        .build();
    state
        .threads
        .thread_store
        .set(
            "thread::target-disabled",
            serde_json::json!({"agent_id": "codex", "workspace_dir": "/tmp/target"}),
        )
        .await
        .unwrap();

    let label_only: UpdateAutomationBody =
        serde_json::from_value(serde_json::json!({"label": "Renamed target"})).unwrap();
    let response = update_automation(
        State(state.clone()),
        Path("automation::target-disabled".to_owned()),
        Json(label_only),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);

    let clear_target: UpdateAutomationBody = serde_json::from_value(serde_json::json!({
        "targetThreadId": null,
        "workspaceDir": "/tmp/generated"
    }))
    .unwrap();
    let response = update_automation(
        State(state.clone()),
        Path("automation::target-disabled".to_owned()),
        Json(clear_target),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        service
            .get("automation::target-disabled")
            .await
            .unwrap()
            .thread_id
            .as_deref(),
        Some("thread::target-disabled")
    );

    // Creating another target-existing automation is pure configuration and
    // ignores the disabled job-level selection entirely.
    let response = create_automation(
        State(state),
        Json(CreateAutomationBody {
            label: "Another target".to_owned(),
            prompt: "Continue it.".to_owned(),
            agent_id: Some("codex".to_owned()),
            workspace_dir: None,
            target_thread_id: Some("thread::target-disabled".to_owned()),
            schedule: AutomationScheduleView::Interval { hours: 1 },
            enabled: Some(true),
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn target_to_generated_uses_live_binding_and_missing_target_requires_explicit_reselection() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(CronService::new(temp.path().to_path_buf()));
    service
        .add(target_automation_config(
            "automation::live-binding",
            "thread::live-binding",
        ))
        .await
        .unwrap();
    service
        .add(target_automation_config(
            "automation::missing-binding",
            "thread::missing-binding",
        ))
        .await
        .unwrap();
    service
        .add(target_automation_config(
            "automation::unbound-target",
            "thread::unbound-target",
        ))
        .await
        .unwrap();
    let state = AppStateBuilder::new(GaryxConfig::default())
        .with_cron_service(service.clone())
        .build();

    // Simulates a legacy unbound thread acquiring its binding later via task
    // assignment: conversion must read this current record, not a boot cache.
    state
        .threads
        .thread_store
        .set(
            "thread::live-binding",
            serde_json::json!({"agent_id": "codex", "workspace_dir": "/tmp/live"}),
        )
        .await
        .unwrap();
    let clear: UpdateAutomationBody = serde_json::from_value(serde_json::json!({
        "targetThreadId": null,
        "workspaceDir": "/tmp/generated"
    }))
    .unwrap();
    let response = update_automation(
        State(state.clone()),
        Path("automation::live-binding".to_owned()),
        Json(clear),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        service
            .get("automation::live-binding")
            .await
            .unwrap()
            .agent_id
            .as_deref(),
        Some("codex")
    );

    let no_choice: UpdateAutomationBody = serde_json::from_value(serde_json::json!({
        "targetThreadId": null,
        "workspaceDir": "/tmp/generated"
    }))
    .unwrap();
    let response = update_automation(
        State(state.clone()),
        Path("automation::missing-binding".to_owned()),
        Json(no_choice),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Existence alone is not a binding. A real legacy thread with no
    // canonical agent must require the same explicit re-selection as a
    // deleted target rather than falling through to the global default.
    state
        .threads
        .thread_store
        .set(
            "thread::unbound-target",
            serde_json::json!({
                "thread_id": "thread::unbound-target",
                "workspace_dir": "/tmp/unbound-target"
            }),
        )
        .await
        .unwrap();
    let no_choice: UpdateAutomationBody = serde_json::from_value(serde_json::json!({
        "targetThreadId": null,
        "workspaceDir": "/tmp/generated"
    }))
    .unwrap();
    let response = update_automation(
        State(state.clone()),
        Path("automation::unbound-target".to_owned()),
        Json(no_choice),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(error["error"].as_str().is_some_and(|message| {
        message.contains("has no agent binding") && message.contains("select an enabled agent")
    }));

    let explicit_unbound: UpdateAutomationBody = serde_json::from_value(serde_json::json!({
        "targetThreadId": null,
        "workspaceDir": "/tmp/generated",
        "agentId": "codex"
    }))
    .unwrap();
    let response = update_automation(
        State(state.clone()),
        Path("automation::unbound-target".to_owned()),
        Json(explicit_unbound),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        service
            .get("automation::unbound-target")
            .await
            .unwrap()
            .agent_id
            .as_deref(),
        Some("codex")
    );

    let explicit: UpdateAutomationBody = serde_json::from_value(serde_json::json!({
        "targetThreadId": null,
        "workspaceDir": "/tmp/generated",
        "agentId": "claude"
    }))
    .unwrap();
    let response = update_automation(
        State(state),
        Path("automation::missing-binding".to_owned()),
        Json(explicit),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn generated_create_omitting_agent_persists_non_claude_effective_default() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(CronService::new(temp.path().to_path_buf()));
    let custom_agents = Arc::new(crate::custom_agents::CustomAgentStore::new());
    custom_agents
        .set_default_agent("codex")
        .await
        .expect("set non-Claude default");
    let state = AppStateBuilder::new(GaryxConfig::default())
        .with_custom_agent_store(custom_agents)
        .with_cron_service(service.clone())
        .build();

    let response = create_automation(
        State(state),
        Json(CreateAutomationBody {
            label: "Defaulted generated".to_owned(),
            prompt: "Use the effective default.".to_owned(),
            agent_id: None,
            workspace_dir: Some("/tmp/defaulted-generated".to_owned()),
            target_thread_id: None,
            schedule: AutomationScheduleView::Interval { hours: 1 },
            enabled: Some(true),
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let summary: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(summary["agentId"], "codex");
    assert_eq!(summary["effectiveAgentId"], "codex");
    assert_eq!(summary["agentResolution"], "resolved");

    let jobs = service.list().await;
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].agent_id.as_deref(), Some("codex"));
}

#[tokio::test]
async fn generated_create_explicit_disabled_agent_is_rejected_without_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(CronService::new(temp.path().to_path_buf()));
    let custom_agents = Arc::new(crate::custom_agents::CustomAgentStore::new());
    custom_agents
        .set_enabled("codex", false)
        .await
        .expect("disable explicit selection");
    let state = AppStateBuilder::new(GaryxConfig::default())
        .with_custom_agent_store(custom_agents)
        .with_cron_service(service.clone())
        .build();

    let response = create_automation(
        State(state),
        Json(CreateAutomationBody {
            label: "Disabled generated".to_owned(),
            prompt: "Do not fall back.".to_owned(),
            agent_id: Some("codex".to_owned()),
            workspace_dir: Some("/tmp/disabled-generated".to_owned()),
            target_thread_id: None,
            schedule: AutomationScheduleView::Interval { hours: 1 },
            enabled: Some(true),
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["error"], "agent is disabled: codex");
    assert!(service.list().await.is_empty());
}

#[tokio::test]
async fn target_to_generated_is_rejected_when_all_agents_are_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(CronService::new(temp.path().to_path_buf()));
    service
        .add(target_automation_config(
            "automation::all-disabled-conversion",
            "thread::all-disabled-conversion",
        ))
        .await
        .unwrap();
    let custom_agents = Arc::new(crate::custom_agents::CustomAgentStore::new());
    for agent in custom_agents.list_agents().await {
        custom_agents
            .set_enabled(&agent.agent_id, false)
            .await
            .expect("disable every agent");
    }
    let state = AppStateBuilder::new(GaryxConfig::default())
        .with_custom_agent_store(custom_agents)
        .with_cron_service(service.clone())
        .build();
    state
        .threads
        .thread_store
        .set(
            "thread::all-disabled-conversion",
            serde_json::json!({
                "thread_id": "thread::all-disabled-conversion",
                "agent_id": "claude",
                "workspace_dir": "/tmp/all-disabled-target"
            }),
        )
        .await
        .unwrap();

    let clear: UpdateAutomationBody = serde_json::from_value(serde_json::json!({
        "targetThreadId": null,
        "workspaceDir": "/tmp/all-disabled-generated"
    }))
    .unwrap();
    let response = update_automation(
        State(state),
        Path("automation::all-disabled-conversion".to_owned()),
        Json(clear),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["error"], "agent is disabled: claude");
    let persisted = service
        .get("automation::all-disabled-conversion")
        .await
        .unwrap();
    assert_eq!(
        persisted.thread_id.as_deref(),
        Some("thread::all-disabled-conversion")
    );
    assert_eq!(persisted.agent_id.as_deref(), Some("claude"));
}

#[tokio::test]
async fn generated_label_edit_preserves_disabled_binding_but_explicit_rebind_rejects() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(CronService::new(temp.path().to_path_buf()));
    let mut generated = target_automation_config("automation::generated-disabled", "unused");
    generated.thread_id = None;
    generated.workspace_dir = Some("/tmp/generated".to_owned());
    generated.agent_id = Some("codex".to_owned());
    service.add(generated).await.unwrap();
    let custom_agents = Arc::new(crate::custom_agents::CustomAgentStore::new());
    custom_agents.set_enabled("codex", false).await.unwrap();
    let state = AppStateBuilder::new(GaryxConfig::default())
        .with_custom_agent_store(custom_agents)
        .with_cron_service(service.clone())
        .build();

    let label_only: UpdateAutomationBody =
        serde_json::from_value(serde_json::json!({"label": "Still bound"})).unwrap();
    let response = update_automation(
        State(state.clone()),
        Path("automation::generated-disabled".to_owned()),
        Json(label_only),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        service
            .get("automation::generated-disabled")
            .await
            .unwrap()
            .agent_id
            .as_deref(),
        Some("codex")
    );

    let explicit_same: UpdateAutomationBody =
        serde_json::from_value(serde_json::json!({"agentId": "codex"})).unwrap();
    let response = update_automation(
        State(state),
        Path("automation::generated-disabled".to_owned()),
        Json(explicit_same),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn generated_create_with_all_agents_disabled_fails_without_creating_job() {
    let temp = tempfile::tempdir().unwrap();
    let service = Arc::new(CronService::new(temp.path().to_path_buf()));
    let custom_agents = Arc::new(crate::custom_agents::CustomAgentStore::new());
    for agent in custom_agents.list_agents().await {
        custom_agents
            .set_enabled(&agent.agent_id, false)
            .await
            .unwrap();
    }
    let state = AppStateBuilder::new(GaryxConfig::default())
        .with_custom_agent_store(custom_agents)
        .with_cron_service(service.clone())
        .build();
    let response = create_automation(
        State(state),
        Json(CreateAutomationBody {
            label: "No agent".to_owned(),
            prompt: "Do work".to_owned(),
            agent_id: None,
            workspace_dir: Some("/tmp/generated".to_owned()),
            target_thread_id: None,
            schedule: AutomationScheduleView::Interval { hours: 1 },
            enabled: Some(true),
        }),
    )
    .await
    .into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(service.list().await.is_empty());
}
