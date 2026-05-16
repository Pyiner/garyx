use super::{
    automation_agent_id, build_automation_job, compile_schedule, infer_schedule_view,
    parse_time_hm, render_data_trigger_template, resolve_automation_agent_id,
    run_data_triggers_for_db_event, to_summary,
};
use crate::app_db::{
    AppDbEvent, AppDbFieldSpec, AppDbService, CreateDataTriggerBody, CreateTableBody,
};
use crate::cron::{CronJob, JobRunStatus};
use crate::server::{AppStateBuilder, create_app_state};
use chrono::Utc;
use garyx_models::config::AutomationScheduleView;
use garyx_models::config::{CronAction, CronJobKind, CronSchedule, GaryxConfig};
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
        "/tmp/repo",
        AutomationScheduleView::Interval { hours: 6 },
        true,
    )
    .expect("automation job");

    assert_eq!(cfg.agent_id.as_deref(), Some("codex"));
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
    };

    assert_eq!(automation_agent_id(&job), "claude");
    let summary = to_summary(&job, None).expect("summary");
    assert_eq!(summary.agent_id, "claude");
}

#[tokio::test]
async fn resolve_automation_agent_id_preserves_raw_team_id() {
    let state = create_app_state(GaryxConfig::default());
    state
        .ops
        .agent_teams
        .upsert_team(crate::agent_teams::UpsertAgentTeamRequest {
            team_id: "product-ship".to_owned(),
            display_name: "Product Ship".to_owned(),
            leader_agent_id: "codex".to_owned(),
            member_agent_ids: vec!["codex".to_owned(), "claude".to_owned()],
            workflow_text: "Codex leads and Claude reviews.".to_owned(),
            avatar_data_url: None,
        })
        .await
        .expect("team saved");

    let resolved = resolve_automation_agent_id(&state, Some("product-ship"), None)
        .await
        .expect("team id should validate");

    assert_eq!(resolved, "product-ship");
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
    config.tasks.enabled = true;
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
