use super::{
    automation_agent_id, build_automation_job, compile_schedule, infer_schedule_view,
    parse_time_hm, resolve_automation_agent_id, to_summary,
};
use crate::cron::{CronJob, JobRunStatus};
use crate::server::create_app_state;
use chrono::Utc;
use garyx_models::config::AutomationScheduleView;
use garyx_models::config::{CronAction, CronJobKind, CronSchedule, GaryxConfig};

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
        })
        .await
        .expect("team saved");

    let resolved = resolve_automation_agent_id(&state, Some("product-ship"), None)
        .await
        .expect("team id should validate");

    assert_eq!(resolved, "product-ship");
}
