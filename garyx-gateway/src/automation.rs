use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{DateTime, Local, Utc};
use garyx_models::config::{
    AutomationScheduleView, CronAction, CronJobConfig, CronJobKind, CronSchedule,
};
use garyx_router::is_thread_key;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::agent_identity::{resolve_agent_reference_from_stores, selected_agent_reference_id};
use crate::cron::{CronJob, JobRunStatus, RunRecord};
use crate::server::AppState;

const AUTOMATION_KEY_PREFIX: &str = "automation::";
pub(crate) const DEFAULT_AUTOMATION_AGENT_ID: &str = "claude";
const DEFAULT_ACTIVITY_LIMIT: usize = 20;
const MAX_ACTIVITY_LIMIT: usize = 100;
const MAX_INTERVAL_HOURS: u64 = (i64::MAX as u64) / 3600;
const WEEKDAY_CODES: [(&str, &str); 7] = [
    ("MON", "mo"),
    ("TUE", "tu"),
    ("WED", "we"),
    ("THU", "th"),
    ("FRI", "fr"),
    ("SAT", "sa"),
    ("SUN", "su"),
];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAutomationBody {
    pub label: String,
    pub prompt: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    pub workspace_dir: String,
    pub schedule: AutomationScheduleView,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAutomationBody {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub schedule: Option<AutomationScheduleView>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Deserialize)]
pub struct ActivityParams {
    #[serde(default = "default_activity_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationSummary {
    pub id: String,
    pub label: String,
    pub prompt: String,
    pub agent_id: String,
    pub enabled: bool,
    pub workspace_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub next_run: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
    pub last_status: JobRunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unread_hint_timestamp: Option<String>,
    pub schedule: AutomationScheduleView,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationActivityEntry {
    pub run_id: String,
    pub status: JobRunStatus,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    pub thread_id: String,
}

fn default_activity_limit() -> usize {
    DEFAULT_ACTIVITY_LIMIT
}

fn invalid(message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": message.into(),
        })),
    )
}

fn not_found(message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "error": message.into(),
        })),
    )
}

fn internal(message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "error": message.into(),
        })),
    )
}

fn service_unavailable(message: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": message.into(),
        })),
    )
}

fn new_automation_id() -> String {
    format!("{AUTOMATION_KEY_PREFIX}{}", Uuid::new_v4())
}

fn trim_required(value: &str, field: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} is required"));
    }
    Ok(trimmed.to_owned())
}

fn parse_time_hm(raw: &str) -> Result<(u8, u8), String> {
    let trimmed = raw.trim();
    let Some((hour_raw, minute_raw)) = trimmed.split_once(':') else {
        return Err("schedule.time must use HH:MM".to_owned());
    };
    let strict_hhmm =
        |part: &str| part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_digit());
    if !strict_hhmm(hour_raw) || !strict_hhmm(minute_raw) {
        return Err("schedule.time must use HH:MM".to_owned());
    }
    let hour = hour_raw
        .parse::<u8>()
        .map_err(|_| "schedule.time hour is invalid".to_owned())?;
    let minute = minute_raw
        .parse::<u8>()
        .map_err(|_| "schedule.time minute is invalid".to_owned())?;
    if hour > 23 || minute > 59 {
        return Err("schedule.time is out of range".to_owned());
    }
    Ok((hour, minute))
}

fn parse_once_input(raw: &str) -> Result<DateTime<Utc>, String> {
    crate::cron::parse_once_timestamp(raw)
        .ok_or_else(|| "schedule.at must use YYYY-MM-DDTHH:MM or ONCE:YYYY-MM-DD HH:MM".to_owned())
}

fn format_once_input(timestamp: DateTime<Utc>) -> String {
    timestamp
        .with_timezone(&Local)
        .format("%Y-%m-%dT%H:%M")
        .to_string()
}

fn normalize_weekday(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "mo" | "mon" | "monday" => Some("MON"),
        "tu" | "tue" | "tuesday" => Some("TUE"),
        "we" | "wed" | "wednesday" => Some("WED"),
        "th" | "thu" | "thursday" => Some("THU"),
        "fr" | "fri" | "friday" => Some("FRI"),
        "sa" | "sat" | "saturday" => Some("SAT"),
        "su" | "sun" | "sunday" => Some("SUN"),
        _ => None,
    }
}

fn weekday_short_code(raw: &str) -> Option<&'static str> {
    let normalized = raw.trim().to_ascii_uppercase();
    WEEKDAY_CODES
        .iter()
        .find_map(|(token, short)| (*token == normalized).then_some(*short))
}

fn expand_weekday_expr(raw: &str) -> Result<Vec<String>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "*" {
        return Ok(Vec::new());
    }

    let mut weekdays = Vec::new();
    for segment in trimmed.split(',') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        if let Some((start, end)) = segment.split_once('-') {
            let start = start.trim().to_ascii_uppercase();
            let end = end.trim().to_ascii_uppercase();
            let Some(start_idx) = WEEKDAY_CODES.iter().position(|(token, _)| *token == start)
            else {
                return Err(format!("unsupported weekday: {segment}"));
            };
            let Some(end_idx) = WEEKDAY_CODES.iter().position(|(token, _)| *token == end) else {
                return Err(format!("unsupported weekday: {segment}"));
            };
            if start_idx > end_idx {
                return Err(format!("unsupported weekday range: {segment}"));
            }
            for (_, short) in WEEKDAY_CODES
                .iter()
                .skip(start_idx)
                .take(end_idx - start_idx + 1)
            {
                if !weekdays.iter().any(|value| value == short) {
                    weekdays.push((*short).to_owned());
                }
            }
            continue;
        }

        let Some(short) = weekday_short_code(segment) else {
            return Err(format!("unsupported weekday: {segment}"));
        };
        if !weekdays.iter().any(|value| value == short) {
            weekdays.push(short.to_owned());
        }
    }

    if weekdays.len() == WEEKDAY_CODES.len() {
        return Ok(Vec::new());
    }

    Ok(weekdays)
}

pub(crate) fn compile_schedule(schedule: &AutomationScheduleView) -> Result<CronSchedule, String> {
    match schedule {
        AutomationScheduleView::Daily {
            time,
            weekdays,
            timezone,
        } => {
            let (hour, minute) = parse_time_hm(time)?;
            let timezone = trim_required(timezone, "schedule.timezone")?;
            let mut weekday_tokens = Vec::new();
            for weekday in weekdays {
                let normalized = normalize_weekday(weekday)
                    .ok_or_else(|| format!("unsupported weekday: {weekday}"))?;
                if !weekday_tokens.contains(&normalized) {
                    weekday_tokens.push(normalized);
                }
            }
            let weekday_expr = if weekday_tokens.is_empty() || weekday_tokens.len() == 7 {
                "*".to_owned()
            } else {
                weekday_tokens.join(",")
            };
            Ok(CronSchedule::Cron {
                expr: format!("0 {minute} {hour} * * {weekday_expr}"),
                timezone: Some(timezone),
            })
        }
        AutomationScheduleView::Interval { hours } => {
            if *hours == 0 {
                return Err("schedule.hours must be greater than 0".to_owned());
            }
            if *hours > MAX_INTERVAL_HOURS {
                return Err(format!(
                    "schedule.hours exceeds max supported value: {MAX_INTERVAL_HOURS}"
                ));
            }
            Ok(CronSchedule::Interval {
                interval_secs: hours * 3600,
            })
        }
        AutomationScheduleView::Once { at } => Ok(CronSchedule::Once {
            at: parse_once_input(at)?.to_rfc3339(),
        }),
    }
}

pub(crate) fn infer_schedule_view(
    schedule: &CronSchedule,
) -> Result<AutomationScheduleView, String> {
    match schedule {
        CronSchedule::Interval { interval_secs } => {
            if *interval_secs == 0 {
                return Err("automation interval must be greater than 0".to_owned());
            }
            if interval_secs % 3600 != 0 {
                return Err(
                    "automation interval must be a whole number of hours to appear in Automation"
                        .to_owned(),
                );
            }
            Ok(AutomationScheduleView::Interval {
                hours: interval_secs / 3600,
            })
        }
        CronSchedule::Once { .. } => {
            let timestamp = crate::cron::parse_once_timestamp(match schedule {
                CronSchedule::Once { at } => at,
                _ => unreachable!(),
            })
            .ok_or_else(|| "automation one-time schedule is invalid".to_owned())?;
            Ok(AutomationScheduleView::Once {
                at: format_once_input(timestamp),
            })
        }
        CronSchedule::Cron { expr, timezone } => {
            let timezone = match timezone.as_deref() {
                Some(value) => trim_required(value, "schedule.timezone")?,
                None => {
                    return Err("automation cron schedules require an explicit timezone".to_owned());
                }
            };
            let parts = expr.split_whitespace().collect::<Vec<_>>();
            if parts.len() != 6 {
                return Err(
                    "automation cron schedules must use `0 MIN HOUR * * WEEKDAYS`".to_owned(),
                );
            }
            if parts[0] != "0" || parts[3] != "*" || parts[4] != "*" {
                return Err(
                    "automation cron schedules must use `0 MIN HOUR * * WEEKDAYS`".to_owned(),
                );
            }
            let minute = parts[1]
                .parse::<u8>()
                .map_err(|_| "automation cron minute is invalid".to_owned())?;
            let hour = parts[2]
                .parse::<u8>()
                .map_err(|_| "automation cron hour is invalid".to_owned())?;
            if hour > 23 || minute > 59 {
                return Err("automation cron time is out of range".to_owned());
            }
            Ok(AutomationScheduleView::Daily {
                time: format!("{hour:02}:{minute:02}"),
                weekdays: expand_weekday_expr(parts[5])?,
                timezone,
            })
        }
    }
}

fn summarize_text(value: Option<&str>, limit: usize) -> Option<String> {
    let text = value?.trim();
    if text.is_empty() {
        return None;
    }
    if text.chars().count() <= limit {
        return Some(text.to_owned());
    }
    Some(
        text.chars()
            .take(limit.saturating_sub(1))
            .collect::<String>()
            .trim_end()
            .to_owned()
            + "…",
    )
}

fn automation_prompt(job: &CronJob) -> String {
    job.message
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_owned()
}

pub(crate) fn automation_agent_id(job: &CronJob) -> String {
    job.agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_AUTOMATION_AGENT_ID)
        .to_owned()
}

pub(crate) async fn resolve_automation_agent_id(
    state: &Arc<AppState>,
    requested: Option<&str>,
    current: Option<&str>,
) -> Result<String, String> {
    let agent_id = selected_agent_reference_id(requested, current);
    resolve_agent_reference_from_stores(
        state.ops.custom_agents.as_ref(),
        state.ops.agent_teams.as_ref(),
        &agent_id,
    )
    .await?;
    Ok(agent_id)
}

fn automation_workspace(job: &CronJob) -> Result<String, String> {
    trim_required(
        job.workspace_dir
            .as_deref()
            .ok_or_else(|| format!("automation {} is missing workspace_dir", job.id))?,
        "workspace_dir",
    )
}

fn latest_automation_thread(latest_run: Option<&RunRecord>) -> Option<String> {
    latest_run
        .and_then(|run| run.thread_id.as_deref())
        .map(str::trim)
        .filter(|value| is_thread_key(value))
        .map(ToOwned::to_owned)
}

fn automation_label(job: &CronJob) -> String {
    job.label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&job.id)
        .to_owned()
}

fn automation_schedule(job: &CronJob) -> Result<AutomationScheduleView, String> {
    if let Some(schedule) = job.ui_schedule.clone() {
        return Ok(schedule);
    }
    infer_schedule_view(&job.schedule)
        .map_err(|error| format!("automation {} has unsupported schedule: {error}", job.id))
}

fn unread_hint_timestamp(job: &CronJob, latest_run: Option<&RunRecord>) -> Option<String> {
    latest_run
        .and_then(|record| record.finished_at)
        .map(|timestamp| timestamp.to_rfc3339())
        .or_else(|| job.last_run_at.map(|timestamp| timestamp.to_rfc3339()))
}

fn automation_next_run(job: &CronJob) -> String {
    match &job.schedule {
        CronSchedule::Once { at } => crate::cron::parse_once_timestamp(at)
            .map(|timestamp| timestamp.to_rfc3339())
            .unwrap_or_else(|| job.next_run.to_rfc3339()),
        _ => job.next_run.to_rfc3339(),
    }
}

fn to_summary(job: &CronJob, latest_run: Option<&RunRecord>) -> Result<AutomationSummary, String> {
    Ok(AutomationSummary {
        id: job.id.clone(),
        label: automation_label(job),
        prompt: automation_prompt(job),
        agent_id: automation_agent_id(job),
        enabled: job.enabled,
        workspace_dir: automation_workspace(job)?,
        thread_id: latest_automation_thread(latest_run),
        next_run: automation_next_run(job),
        last_run_at: job.last_run_at.map(|value| value.to_rfc3339()),
        last_status: job.last_status.clone(),
        unread_hint_timestamp: unread_hint_timestamp(job, latest_run),
        schedule: automation_schedule(job)?,
    })
}

async fn excerpt_for_run(
    state: &Arc<AppState>,
    thread_id: Option<&str>,
    run: &RunRecord,
) -> Option<String> {
    let thread_id = thread_id?.trim();
    if thread_id.is_empty() {
        return None;
    }
    let messages = state
        .threads
        .history
        .find_latest_for_run(thread_id, &run.run_id)
        .await
        .ok()?;

    for role in ["assistant", "user"] {
        for message in messages.iter().rev() {
            if message.get("role").and_then(Value::as_str) != Some(role) {
                continue;
            }
            if let Some(summary) =
                summarize_text(message.get("content").and_then(Value::as_str), 220)
            {
                return Some(summary);
            }
            if let Some(summary) = summarize_text(message.get("text").and_then(Value::as_str), 220)
            {
                return Some(summary);
            }
        }
    }

    None
}

async fn to_activity_entry(
    state: &Arc<AppState>,
    thread_id: Option<&str>,
    run: &RunRecord,
) -> AutomationActivityEntry {
    AutomationActivityEntry {
        run_id: run.run_id.clone(),
        status: run.status.clone(),
        started_at: run.started_at.to_rfc3339(),
        finished_at: run.finished_at.map(|value| value.to_rfc3339()),
        duration_ms: run.duration_ms,
        excerpt: excerpt_for_run(state, thread_id, run).await,
        thread_id: thread_id.unwrap_or_default().to_owned(),
    }
}

fn is_automation_job(job: &CronJob) -> bool {
    job.kind == CronJobKind::AutomationPrompt
        && job.action == CronAction::AgentTurn
        && job
            .message
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        && job
            .workspace_dir
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
}

async fn cron_service(
    state: &Arc<AppState>,
) -> Result<Arc<crate::cron::CronService>, (StatusCode, Json<Value>)> {
    state
        .ops
        .cron_service
        .clone()
        .ok_or_else(|| service_unavailable("automation service unavailable"))
}

async fn automation_job(
    state: &Arc<AppState>,
    automation_id: &str,
) -> Result<(Arc<crate::cron::CronService>, CronJob), (StatusCode, Json<Value>)> {
    let service = cron_service(state).await?;
    let Some(job) = service.get(automation_id).await else {
        return Err(not_found("automation not found"));
    };
    if !is_automation_job(&job) {
        return Err(not_found("automation not found"));
    }
    Ok((service, job))
}

pub(crate) fn build_automation_job(
    automation_id: &str,
    label: &str,
    prompt: &str,
    agent_id: &str,
    workspace_dir: &str,
    schedule: AutomationScheduleView,
    enabled: bool,
) -> Result<CronJobConfig, String> {
    Ok(CronJobConfig {
        id: automation_id.to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some(label.to_owned()),
        schedule: compile_schedule(&schedule)?,
        ui_schedule: Some(schedule),
        action: CronAction::AgentTurn,
        target: None,
        message: Some(prompt.to_owned()),
        workspace_dir: Some(workspace_dir.to_owned()),
        agent_id: Some(agent_id.to_owned()),
        thread_id: None,
        delete_after_run: false,
        enabled,
    })
}

pub async fn list_automations(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let service = match cron_service(&state).await {
        Ok(service) => service,
        Err(error) => return error,
    };

    let jobs = service.list().await;
    let all_runs = service.list_runs(500, 0).await;
    let mut summaries = Vec::new();
    for job in jobs.into_iter().filter(is_automation_job) {
        let latest_run = all_runs.iter().find(|run| run.job_id == job.id);
        match to_summary(&job, latest_run) {
            Ok(summary) => summaries.push(summary),
            Err(error) => return internal(error),
        }
    }

    summaries.sort_by(|left, right| {
        left.next_run
            .cmp(&right.next_run)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.id.cmp(&right.id))
    });

    (StatusCode::OK, Json(json!({ "automations": summaries })))
}

pub async fn get_automation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let (service, job) = match automation_job(&state, &id).await {
        Ok(value) => value,
        Err(error) => return error,
    };
    let latest_run = service.list_runs_for_job(&job.id, 1, 0).await;
    match to_summary(&job, latest_run.first()) {
        Ok(summary) => (StatusCode::OK, Json(json!(summary))),
        Err(error) => internal(error),
    }
}

pub async fn create_automation(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateAutomationBody>,
) -> impl IntoResponse {
    let service = match cron_service(&state).await {
        Ok(service) => service,
        Err(error) => return error,
    };

    let label = match trim_required(&body.label, "label") {
        Ok(value) => value,
        Err(error) => return invalid(error),
    };
    let prompt = match trim_required(&body.prompt, "prompt") {
        Ok(value) => value,
        Err(error) => return invalid(error),
    };
    let agent_id = match resolve_automation_agent_id(&state, body.agent_id.as_deref(), None).await {
        Ok(value) => value,
        Err(error) => return invalid(error),
    };
    let workspace_dir = match trim_required(&body.workspace_dir, "workspace_dir") {
        Ok(value) => value,
        Err(error) => return invalid(error),
    };

    let automation_id = new_automation_id();
    let cfg = match build_automation_job(
        &automation_id,
        &label,
        &prompt,
        &agent_id,
        &workspace_dir,
        body.schedule.clone(),
        true,
    ) {
        Ok(cfg) => cfg,
        Err(error) => return invalid(error),
    };

    let job = match service.add(cfg).await {
        Ok(job) => job,
        Err(error) => return invalid(error.to_string()),
    };

    let summary = match to_summary(&job, None) {
        Ok(summary) => summary,
        Err(error) => return internal(error),
    };

    (StatusCode::CREATED, Json(json!(summary)))
}

pub async fn update_automation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateAutomationBody>,
) -> impl IntoResponse {
    let (service, current) = match automation_job(&state, &id).await {
        Ok(value) => value,
        Err(error) => return error,
    };

    let label = match body.label.as_deref() {
        Some(value) => match trim_required(value, "label") {
            Ok(value) => value,
            Err(error) => return invalid(error),
        },
        None => automation_label(&current),
    };
    let prompt = match body.prompt.as_deref() {
        Some(value) => match trim_required(value, "prompt") {
            Ok(value) => value,
            Err(error) => return invalid(error),
        },
        None => automation_prompt(&current),
    };
    let agent_id = match resolve_automation_agent_id(
        &state,
        body.agent_id.as_deref(),
        current.agent_id.as_deref(),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => return invalid(error),
    };
    let workspace_dir = match body.workspace_dir.as_deref() {
        Some(value) => match trim_required(value, "workspace_dir") {
            Ok(value) => value,
            Err(error) => return invalid(error),
        },
        None => match automation_workspace(&current) {
            Ok(value) => value,
            Err(error) => return internal(error),
        },
    };
    let schedule = match body.schedule.clone() {
        Some(value) => value,
        None => match automation_schedule(&current) {
            Ok(value) => value,
            Err(error) => return internal(error),
        },
    };
    let enabled = body.enabled.unwrap_or(current.enabled);

    let cfg = match build_automation_job(
        &id,
        &label,
        &prompt,
        &agent_id,
        &workspace_dir,
        schedule,
        enabled,
    ) {
        Ok(cfg) => cfg,
        Err(error) => return invalid(error),
    };

    let Some(job) = (match service.update(&id, cfg).await {
        Ok(value) => value,
        Err(error) => return invalid(error.to_string()),
    }) else {
        return not_found("automation not found");
    };

    let latest_run = service.list_runs_for_job(&job.id, 1, 0).await;
    match to_summary(&job, latest_run.first()) {
        Ok(summary) => (StatusCode::OK, Json(json!(summary))),
        Err(error) => internal(error),
    }
}

pub async fn delete_automation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let (service, _job) = match automation_job(&state, &id).await {
        Ok(value) => value,
        Err(error) => return error,
    };

    match service.delete(&id).await {
        Ok(true) => {}
        Ok(false) => return not_found("automation not found"),
        Err(error) => return internal(error.to_string()),
    }

    (
        StatusCode::OK,
        Json(json!({
            "deleted": true,
            "id": id,
        })),
    )
}

pub async fn run_automation_now(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let (service, _job) = match automation_job(&state, &id).await {
        Ok(value) => value,
        Err(error) => return error,
    };

    let Some(run) = service.run_now(&id).await else {
        return invalid("automation is disabled or missing");
    };
    let thread_id = run.thread_id.clone();
    (
        StatusCode::OK,
        Json(json!(
            to_activity_entry(&state, thread_id.as_deref(), &run).await
        )),
    )
}

pub async fn automation_activity(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<ActivityParams>,
) -> impl IntoResponse {
    let (service, _job) = match automation_job(&state, &id).await {
        Ok(value) => value,
        Err(error) => return error,
    };
    let limit = params.limit.clamp(1, MAX_ACTIVITY_LIMIT);
    let runs = service.list_runs_for_job(&id, limit, params.offset).await;
    let latest_thread_id = latest_automation_thread(runs.first());
    let mut items = Vec::with_capacity(runs.len());
    for run in &runs {
        let thread_id = run.thread_id.clone().or_else(|| latest_thread_id.clone());
        items.push(to_activity_entry(&state, thread_id.as_deref(), run).await);
    }

    (
        StatusCode::OK,
        Json(json!({
            "items": items,
            "threadId": latest_thread_id,
            "count": items.len(),
        })),
    )
}

#[cfg(test)]
mod tests;
