use super::{
    AutomationThreadsParams, UpdateAutomationBody, automation_agent_id, automation_threads,
    build_automation_job, compile_schedule, infer_schedule_view, is_automation_job, parse_time_hm,
    render_data_trigger_template, run_data_triggers_for_db_event, to_summary, update_automation,
};
use crate::app_db::{
    AppDbEvent, AppDbFieldSpec, AppDbService, CreateDataTriggerBody, CreateTableBody,
};
use crate::cron::{CronJob, CronService, JobRunStatus};
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
use garyx_models::{Principal, TaskStatus, ThreadTask};
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
        "codex",
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
        "codex",
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
    };

    assert!(is_automation_job(&job));
}

#[test]
fn automation_summary_defaults_agent_to_claude_for_legacy_jobs() {
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
    };

    assert_eq!(automation_agent_id(&job), "claude");
    let summary = to_summary(&job, None).expect("summary");
    assert_eq!(summary.agent_id, "claude");
}

#[test]
fn automation_summary_exposes_bound_target_thread() {
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
    };

    let summary = to_summary(&job, None).expect("summary");
    assert_eq!(summary.target_thread_id.as_deref(), Some("thread::target"));
    assert_eq!(summary.thread_id.as_deref(), Some("thread::target"));
    assert_eq!(summary.workspace_dir, "");
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
        .await;
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
                "exclude_from_recent": true,
                "messages": [{"role": "user", "content": "Summarize."}]
            }),
        )
        .await;
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
    assert_eq!(payload["items"][0]["thread"]["excludeFromRecent"], true);
}

#[test]
fn data_trigger_template_renders_db_event_fields() {
    let event = AppDbEvent {
        id: "evt_test".to_owned(),
        event_type: "record.created".to_owned(),
        table_name: "contacts".to_owned(),
        record_id: Some("rec_test".to_owned()),
        actor_type: None,
        actor_id: None,
        thread_id: None,
        task_id: None,
        schema_version: None,
        before: None,
        after: None,
        created_at: "2030-05-01T08:30:00Z".to_owned(),
    };

    assert_eq!(
        render_data_trigger_template(
            "Handle {event_type} on {table_name}/{record_id} ({event_id})",
            &event
        ),
        "Handle record.created on contacts/rec_test (evt_test)"
    );
}

#[tokio::test]
async fn data_trigger_with_agent_id_creates_and_dispatches_agent_task() {
    let temp = tempfile::tempdir().unwrap();
    let app_db = Arc::new(
        AppDbService::open(temp.path().join("app.sqlite3")).expect("app db opens for test"),
    );
    app_db
        .create_table(
            CreateTableBody {
                table_name: "contacts".to_owned(),
                display_name: None,
                fields: vec![AppDbFieldSpec {
                    name: "name".to_owned(),
                    field_type: "TEXT".to_owned(),
                    not_null: false,
                    unique: false,
                    indexed: false,
                    display_name: None,
                    default_value: None,
                }],
            },
            None,
        )
        .expect("table created");
    let trigger = app_db
        .create_data_trigger(CreateDataTriggerBody {
            label: "Contact review".to_owned(),
            table_name: "contacts".to_owned(),
            event_type: "record.created".to_owned(),
            title_template: "Review {record_id}".to_owned(),
            body_template: "Handle {table_name}/{record_id}".to_owned(),
            agent_id: Some("codex".to_owned()),
            workspace_dir: Some(temp.path().join("workspace").to_string_lossy().to_string()),
            enabled: true,
        })
        .expect("trigger created");
    let mut config = GaryxConfig::default();
    config.sessions.data_dir = Some(temp.path().join("data").to_string_lossy().to_string());
    let state = AppStateBuilder::new(config).with_app_db(app_db).build();
    let event = AppDbEvent {
        id: "evt_test".to_owned(),
        event_type: "record.created".to_owned(),
        table_name: "contacts".to_owned(),
        record_id: Some("rec_test".to_owned()),
        actor_type: None,
        actor_id: None,
        thread_id: None,
        task_id: None,
        schema_version: None,
        before: None,
        after: None,
        created_at: "2030-05-01T08:30:00Z".to_owned(),
    };

    let results = run_data_triggers_for_db_event(state.clone(), &event).await;

    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert_eq!(result["triggerId"], trigger.id);
    assert_eq!(result["status"], "created");
    assert_eq!(result["dispatch"]["queued"], true);
    assert_eq!(result["dispatch"]["agent_id"], "codex");
    let thread_id = result["threadId"].as_str().expect("thread id");
    let record = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("task thread exists");
    assert_eq!(record["agent_id"], "codex");
    assert_eq!(record["provider_type"], "codex_app_server");
    let task: ThreadTask = serde_json::from_value(record["task"].clone()).expect("task record");
    assert_eq!(task.status, TaskStatus::InProgress);
    assert_eq!(
        task.assignee,
        Some(Principal::Agent {
            agent_id: "codex".to_owned()
        })
    );
    assert_eq!(task.title, "Review rec_test");
}
