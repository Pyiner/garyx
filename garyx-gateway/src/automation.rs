use garyx_router::ThreadStoreExt;
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
use garyx_router::{history_message_count, is_thread_key, workspace_dir_from_value};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::agent_identity::resolve_new_agent_binding_from_store;
use crate::cron::{CronJob, JobRunStatus, RunRecord};
use crate::garyx_db::AutomationThreadRunRecord;
use crate::server::AppState;
use crate::thread_type::thread_summary_type_from_record;
use crate::transcript_run_projection::active_run_id_from_transcript_store;

const AUTOMATION_KEY_PREFIX: &str = "automation::";
pub(crate) const DEFAULT_AUTOMATION_AGENT_ID: &str = "claude";
const DEFAULT_ACTIVITY_LIMIT: usize = 20;
const MAX_ACTIVITY_LIMIT: usize = 100;
const DEFAULT_AUTOMATION_THREADS_LIMIT: usize = 50;
const MAX_AUTOMATION_THREADS_LIMIT: usize = 100;
/// Upper bound on interval schedules, in hours (100 years). Mirrors the cron
/// layer's `MAX_INTERVAL_SECS` so an over-large interval is rejected cleanly
/// here instead of overflowing chrono's `DateTime` math downstream.
const MAX_INTERVAL_HOURS: u64 = 100 * 365 * 24;
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
    #[serde(default)]
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub target_thread_id: Option<String>,
    pub schedule: AutomationScheduleView,
    #[serde(default)]
    pub enabled: Option<bool>,
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
    #[serde(default, deserialize_with = "deserialize_present_option")]
    pub target_thread_id: Option<Option<String>>,
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

#[derive(Deserialize)]
pub struct AutomationThreadsParams {
    #[serde(default = "default_automation_threads_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationSummary {
    pub id: String,
    pub label: String,
    pub prompt: String,
    pub agent_id: Option<String>,
    pub agent_resolution: AutomationAgentResolution,
    pub effective_agent_id: Option<String>,
    pub enabled: bool,
    pub workspace_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub thread_mode: String,
    pub next_run: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
    pub last_status: JobRunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unread_hint_timestamp: Option<String>,
    pub schedule: AutomationScheduleView,
    pub validation_state: AutomationValidationState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationAgentResolution {
    Resolved,
    FollowThread,
    TargetMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationValidationState {
    Valid,
    Invalid,
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

fn default_automation_threads_limit() -> usize {
    DEFAULT_AUTOMATION_THREADS_LIMIT
}

fn deserialize_present_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
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

fn parse_month_day(day: u8) -> Result<u8, String> {
    if (1..=31).contains(&day) {
        Ok(day)
    } else {
        Err("schedule.day must be between 1 and 31".to_owned())
    }
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
        AutomationScheduleView::Monthly {
            day,
            time,
            timezone,
        } => {
            let day = parse_month_day(*day)?;
            let (hour, minute) = parse_time_hm(time)?;
            let timezone = trim_required(timezone, "schedule.timezone")?;
            Ok(CronSchedule::Cron {
                expr: format!("0 {minute} {hour} {day} * *"),
                timezone: Some(timezone),
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
                    "automation cron schedules must use `0 MIN HOUR * * WEEKDAYS` or `0 MIN HOUR DAY * *`"
                        .to_owned(),
                );
            }
            if parts[0] != "0" || parts[4] != "*" {
                return Err(
                    "automation cron schedules must use `0 MIN HOUR * * WEEKDAYS` or `0 MIN HOUR DAY * *`"
                        .to_owned(),
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
            if parts[3] == "*" {
                return Ok(AutomationScheduleView::Daily {
                    time: format!("{hour:02}:{minute:02}"),
                    weekdays: expand_weekday_expr(parts[5])?,
                    timezone,
                });
            }
            if parts[5] != "*" {
                return Err(
                    "automation monthly cron schedules must use `0 MIN HOUR DAY * *`".to_owned(),
                );
            }
            let day = parts[3]
                .parse::<u8>()
                .map_err(|_| "automation cron day is invalid".to_owned())
                .and_then(parse_month_day)?;
            Ok(AutomationScheduleView::Monthly {
                day,
                time: format!("{hour:02}:{minute:02}"),
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

fn last_thread_message_preview(data: &Value, role: &str) -> Option<String> {
    // Write-time preview fields are the source (#TASK-1864 batch 1).
    if let Some(preview) = garyx_models::message_preview::preview_field_for_role(role)
        .and_then(|field| data.get(field))
        .and_then(Value::as_str)
    {
        return Some(preview.to_owned());
    }
    None
}

fn automation_prompt(job: &CronJob) -> String {
    job.message
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
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
    garyx_models::resolve_agent_binding(
        &state.ops.custom_agents.snapshot().await,
        requested,
        current,
    )
    .map(|binding| binding.agent_id)
    .map_err(|error| error.to_string())
}

fn automation_workspace(job: &CronJob) -> Result<String, String> {
    let workspace_dir = job
        .workspace_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(workspace_dir) = workspace_dir {
        return Ok(workspace_dir.to_owned());
    }
    if automation_target_thread(job).is_some()
        || job
            .target
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
    {
        return Ok(String::new());
    }
    if job.validation_error.is_some() {
        return Ok(String::new());
    }
    Err(format!("automation {} is missing workspace_dir", job.id))
}

fn automation_target_thread(job: &CronJob) -> Option<String> {
    job.thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| is_thread_key(value))
        .map(ToOwned::to_owned)
}

fn latest_automation_thread(latest_run: Option<&RunRecord>) -> Option<String> {
    latest_run
        .and_then(|run| run.thread_id.as_deref())
        .map(str::trim)
        .filter(|value| is_thread_key(value))
        .map(ToOwned::to_owned)
}

fn automation_thread_mode(job: &CronJob) -> String {
    if automation_target_thread(job).is_some() {
        "target".to_owned()
    } else {
        "generated".to_owned()
    }
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

/// User-managed automations normally persist `ui_schedule`, but legacy cron
/// rows may use a valid schedule that the automation editor cannot infer
/// losslessly. Keep such rows visible and repairable by presenting their next
/// fire as a one-time fallback. Update paths that omit `schedule` preserve the
/// original raw schedule instead of compiling this display-only fallback.
fn automation_schedule_for_display(job: &CronJob) -> AutomationScheduleView {
    automation_schedule(job).unwrap_or_else(|_| AutomationScheduleView::Once {
        at: automation_next_run(job),
    })
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

async fn to_summary(
    state: &Arc<AppState>,
    job: &CronJob,
    latest_run: Option<&RunRecord>,
) -> Result<AutomationSummary, String> {
    let target_thread_id = automation_target_thread(job);
    let (agent_id, agent_resolution, effective_agent_id) =
        if let Some(thread_id) = target_thread_id.as_deref() {
            match state
                .threads
                .thread_store
                .get(thread_id)
                .await
                .map_err(|error| error.to_string())?
            {
                Some(thread) => (
                    None,
                    AutomationAgentResolution::FollowThread,
                    garyx_router::agent_id_from_value(&thread),
                ),
                None => (None, AutomationAgentResolution::TargetMissing, None),
            }
        } else {
            // Valid generated jobs are normalized to an explicit agent. An
            // invalid legacy thread-less row remains visible and repairable;
            // do not manufacture a Claude identity into its typed wire state.
            let agent_id = job
                .agent_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .or_else(|| {
                    job.workspace_dir
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|_| DEFAULT_AUTOMATION_AGENT_ID.to_owned())
                });
            (
                agent_id.clone(),
                AutomationAgentResolution::Resolved,
                agent_id,
            )
        };
    Ok(AutomationSummary {
        id: job.id.clone(),
        label: automation_label(job),
        prompt: automation_prompt(job),
        agent_id,
        agent_resolution,
        effective_agent_id,
        enabled: job.enabled,
        workspace_dir: automation_workspace(job)?,
        target_thread_id: target_thread_id.clone(),
        thread_id: target_thread_id.or_else(|| latest_automation_thread(latest_run)),
        thread_mode: automation_thread_mode(job),
        next_run: automation_next_run(job),
        last_run_at: job.last_run_at.map(|value| value.to_rfc3339()),
        last_status: job.last_status.clone(),
        unread_hint_timestamp: unread_hint_timestamp(job, latest_run),
        schedule: automation_schedule_for_display(job),
        validation_state: if job.validation_error.is_some() {
            AutomationValidationState::Invalid
        } else {
            AutomationValidationState::Valid
        },
        validation_error: job.validation_error.clone(),
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
}

#[derive(Clone)]
struct AutomationTargetThread {
    thread_id: String,
    workspace_dir: Option<String>,
    agent_id: Option<String>,
    exists: bool,
}

async fn resolve_automation_target_thread(
    state: &Arc<AppState>,
    candidate: Option<&str>,
) -> Result<Option<AutomationTargetThread>, String> {
    let Some(thread_id) = candidate.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if !is_thread_key(thread_id) {
        return Err("targetThreadId must be an existing thread id".to_owned());
    }
    let thread_data = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(Some(AutomationTargetThread {
        thread_id: thread_id.to_owned(),
        workspace_dir: thread_data.as_ref().and_then(workspace_dir_from_value),
        agent_id: thread_data
            .as_ref()
            .and_then(garyx_router::agent_id_from_value),
        exists: thread_data.is_some(),
    }))
}

fn resolve_automation_workspace_input(
    workspace_dir: Option<&str>,
    target_thread: Option<&AutomationTargetThread>,
    fallback_workspace_dir: Option<&str>,
) -> Result<String, String> {
    if let Some(workspace_dir) = workspace_dir
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        // A thread-bound automation runs like a person sending a message
        // into that thread: the thread's own workspace always applies, so a
        // conflicting explicit workspace would silently be ignored — reject
        // the combination instead.
        if target_thread.is_some() {
            return Err(
                "workspace_dir cannot be combined with targetThreadId; a thread-bound \
                 automation always uses the thread's workspace"
                    .to_owned(),
            );
        }
        return Ok(workspace_dir.to_owned());
    }
    if let Some(fallback_workspace_dir) = fallback_workspace_dir
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(fallback_workspace_dir.to_owned());
    }
    if let Some(target_workspace_dir) = target_thread
        .and_then(|target| target.workspace_dir.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(target_workspace_dir.to_owned());
    }
    if target_thread.is_some() {
        return Ok(String::new());
    }
    Err("workspace_dir is required unless targetThreadId is set".to_owned())
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_automation_job(
    automation_id: &str,
    label: &str,
    prompt: &str,
    agent_id: Option<&str>,
    workspace_dir: Option<&str>,
    target_thread_id: Option<&str>,
    schedule: AutomationScheduleView,
    enabled: bool,
) -> Result<CronJobConfig, String> {
    let workspace_dir = workspace_dir
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let target_thread_id = target_thread_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    Ok(CronJobConfig {
        id: automation_id.to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some(label.to_owned()),
        schedule: compile_schedule(&schedule)?,
        ui_schedule: Some(schedule),
        action: CronAction::AgentTurn,
        target: None,
        message: Some(prompt.to_owned()),
        workspace_dir,
        agent_id: agent_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        thread_id: target_thread_id,
        delete_after_run: false,
        enabled,
        system: false,
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
        match to_summary(&state, &job, latest_run).await {
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
    match to_summary(&state, &job, latest_run.first()).await {
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
    let target_thread =
        match resolve_automation_target_thread(&state, body.target_thread_id.as_deref()).await {
            Ok(value) => value,
            Err(error) => return invalid(error),
        };
    if target_thread.as_ref().is_some_and(|target| !target.exists) {
        return invalid(format!(
            "target thread not found: {}",
            target_thread.as_ref().unwrap().thread_id
        ));
    }
    // A thread-bound automation executes under the thread's own agent, so
    // the automation-level agent is not validated (and a stale/deleted one
    // must not block unrelated edits). Generated-thread automations still
    // require a resolvable agent.
    let agent_id = if target_thread.is_some() {
        None
    } else {
        match resolve_automation_agent_id(&state, body.agent_id.as_deref(), None).await {
            Ok(value) => Some(value),
            Err(error) => return invalid(error),
        }
    };
    let explicit_workspace_dir = body
        .workspace_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let workspace_dir = match resolve_automation_workspace_input(
        body.workspace_dir.as_deref(),
        target_thread.as_ref(),
        None,
    ) {
        Ok(value) => value,
        Err(error) => return invalid(error),
    };
    let job_workspace_dir = if target_thread.is_some() && explicit_workspace_dir.is_none() {
        None
    } else {
        Some(workspace_dir.as_str())
    };

    let automation_id = new_automation_id();
    let cfg = match build_automation_job(
        &automation_id,
        &label,
        &prompt,
        agent_id.as_deref(),
        job_workspace_dir,
        target_thread
            .as_ref()
            .map(|target| target.thread_id.as_str()),
        body.schedule.clone(),
        body.enabled.unwrap_or(true),
    ) {
        Ok(cfg) => cfg,
        Err(error) => return invalid(error),
    };

    let job = match service.add(cfg).await {
        Ok(job) => job,
        Err(error) => return invalid(error.to_string()),
    };

    let summary = match to_summary(&state, &job, None).await {
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
    let current_target_id = automation_target_thread(&current);
    let current_target =
        match resolve_automation_target_thread(&state, current_target_id.as_deref()).await {
            Ok(value) => value,
            Err(error) => return invalid(error),
        };
    let target_thread = match &body.target_thread_id {
        Some(Some(thread_id)) => {
            match resolve_automation_target_thread(&state, Some(thread_id.as_str())).await {
                Ok(value) => value,
                Err(error) => return invalid(error),
            }
        }
        Some(None) => None,
        None => current_target.clone(),
    };
    if body.target_thread_id.as_ref().is_some_and(Option::is_some)
        && target_thread.as_ref().is_some_and(|target| !target.exists)
    {
        return invalid(format!(
            "target thread not found: {}",
            target_thread.as_ref().unwrap().thread_id
        ));
    }
    // A thread-bound automation executes under the thread's own agent: skip
    // job-agent validation so a stale/deleted automation agent cannot 400 an
    // unrelated edit (e.g. renaming the label).
    let agent_id = if target_thread.is_some() {
        None
    } else if let Some(requested) = body
        .agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        match resolve_new_agent_binding_from_store(
            state.ops.custom_agents.as_ref(),
            Some(requested),
        )
        .await
        {
            Ok(binding) => Some(binding.agent_id),
            Err(error) => return invalid(error.to_string()),
        }
    } else if current_target_id.is_some() {
        let Some(current_agent_id) = current_target
            .as_ref()
            .filter(|target| target.exists)
            .and_then(|target| target.agent_id.as_deref())
        else {
            return invalid(
                "target thread is missing or has no agent binding; select an enabled agent"
                    .to_owned(),
            );
        };
        match resolve_new_agent_binding_from_store(
            state.ops.custom_agents.as_ref(),
            Some(current_agent_id),
        )
        .await
        {
            Ok(binding) => Some(binding.agent_id),
            Err(error) => return invalid(error.to_string()),
        }
    } else {
        match resolve_automation_agent_id(
            &state,
            body.agent_id.as_deref(),
            current.agent_id.as_deref(),
        )
        .await
        {
            Ok(value) => Some(value),
            Err(error) => return invalid(error),
        }
    };
    let explicit_workspace_dir = body
        .workspace_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let target_thread_changed = body.target_thread_id.is_some();
    let fallback_workspace_dir = if target_thread_changed {
        target_thread
            .as_ref()
            .and_then(|target| target.workspace_dir.as_deref())
            .or_else(|| {
                current_target
                    .as_ref()
                    .and_then(|target| target.workspace_dir.as_deref())
            })
            .or(current.workspace_dir.as_deref())
    } else {
        current.workspace_dir.as_deref()
    };
    let workspace_dir = match resolve_automation_workspace_input(
        body.workspace_dir.as_deref(),
        target_thread.as_ref(),
        fallback_workspace_dir,
    ) {
        Ok(value) => value,
        Err(error) => return invalid(error),
    };
    let job_workspace_dir = if target_thread.is_some() && explicit_workspace_dir.is_none() {
        None
    } else {
        Some(workspace_dir.as_str())
    };
    let (schedule, preserve_raw_schedule) = match body.schedule.clone() {
        Some(value) => (value, false),
        None => (automation_schedule_for_display(&current), true),
    };
    let enabled = body.enabled.unwrap_or(current.enabled);

    let mut cfg = match build_automation_job(
        &id,
        &label,
        &prompt,
        agent_id.as_deref(),
        job_workspace_dir,
        target_thread
            .as_ref()
            .map(|target| target.thread_id.as_str()),
        schedule,
        enabled,
    ) {
        Ok(cfg) => cfg,
        Err(error) => return invalid(error),
    };
    if preserve_raw_schedule {
        cfg.schedule = current.schedule.clone();
        cfg.ui_schedule = current.ui_schedule.clone();
    }

    let Some(job) = (match service.update(&id, cfg).await {
        Ok(value) => value,
        Err(error) => return invalid(error.to_string()),
    }) else {
        return not_found("automation not found");
    };

    let latest_run = service.list_runs_for_job(&job.id, 1, 0).await;
    match to_summary(&state, &job, latest_run.first()).await {
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

    let env = crate::automation_wiring::automation_exec_env(&state);
    let Some(run) = service.run_now(&id, &env).await else {
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

fn normalize_automation_thread_mode_param(value: Option<&str>) -> Result<&'static str, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("generated") | Some("generated_thread") => Ok("generated_thread"),
        Some("target") | Some("target_thread") => Ok("target_thread"),
        Some(_) => Err("mode must be generated or target".to_owned()),
    }
}

fn automation_thread_summary(
    thread_id: &str,
    data: &Value,
    active_run_id: Option<String>,
) -> Value {
    let title = data
        .get("label")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("New Thread");
    let recent_run_id = data
        .get("history")
        .and_then(|history| history.get("recent_committed_run_ids"))
        .and_then(Value::as_array)
        .and_then(|entries| entries.last())
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    json!({
        "id": thread_id,
        "threadId": thread_id,
        "threadType": thread_summary_type_from_record(data),
        "title": title,
        "label": title,
        "workspaceDir": workspace_dir_from_value(data),
        "agentId": data.get("agent_id").and_then(Value::as_str),
        "providerType": data.get("provider_type").and_then(Value::as_str),
        "messageCount": history_message_count(data),
        "lastUserMessage": last_thread_message_preview(data, "user"),
        "lastAssistantMessage": last_thread_message_preview(data, "assistant"),
        "recentRunId": recent_run_id,
        "activeRunId": active_run_id,
        "createdAt": data.get("created_at").and_then(Value::as_str),
        "updatedAt": data.get("updated_at").and_then(Value::as_str),
        "automationId": data.get("automation_id").and_then(Value::as_str),
        "automationThreadMode": data.get("automation_thread_mode").and_then(Value::as_str),
    })
}

async fn automation_thread_entry(
    state: &Arc<AppState>,
    record: &AutomationThreadRunRecord,
    automation_label: Option<&str>,
    automation_deleted: bool,
) -> Value {
    let thread = state
        .threads
        .thread_store
        .get_logged(&record.thread_id)
        .await;
    let thread = match thread {
        Some(data) => {
            let transcript_store = state.threads.history.transcript_store();
            let active_run_id =
                active_run_id_from_transcript_store(&transcript_store, &record.thread_id).await;
            Some(automation_thread_summary(
                &record.thread_id,
                &data,
                active_run_id,
            ))
        }
        None => None,
    };
    json!({
        "automationId": record.automation_id.as_str(),
        "runId": record.run_id.as_str(),
        "threadId": record.thread_id.as_str(),
        "workspaceDir": record.workspace_dir.as_deref(),
        "agentId": record.agent_id.as_deref(),
        "automationLabel": automation_label
            .map(ToOwned::to_owned)
            .or_else(|| record.automation_label_snapshot.clone())
            .unwrap_or_else(|| record.automation_id.clone()),
        "automationDeleted": automation_deleted,
        "mode": record.mode.as_str(),
        "status": record.status.as_str(),
        "startedAt": record.started_at.as_str(),
        "finishedAt": record.finished_at.as_deref(),
        "thread": thread,
    })
}

pub async fn automation_threads(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<AutomationThreadsParams>,
) -> impl IntoResponse {
    let mode = match normalize_automation_thread_mode_param(params.mode.as_deref()) {
        Ok(mode) => mode,
        Err(message) => return invalid(message),
    };
    let limit = params.limit.clamp(1, MAX_AUTOMATION_THREADS_LIMIT);
    let offset = params.offset;
    let service = state.ops.cron_service.as_ref().cloned();
    let job = if let Some(service) = service {
        service.get(&id).await.filter(is_automation_job)
    } else {
        None
    };
    let label = job.as_ref().map(automation_label);
    let automation_deleted = job.is_none();
    let total = match state
        .ops
        .garyx_db
        .count_automation_thread_runs(&id, Some(mode))
    {
        Ok(total) => total,
        Err(error) => return internal(format!("failed to count automation threads: {error}")),
    };
    if job.is_none() && total == 0 {
        return not_found("automation not found");
    }
    let records =
        match state
            .ops
            .garyx_db
            .list_automation_thread_runs(&id, Some(mode), limit, offset)
        {
            Ok(records) => records,
            Err(error) => return internal(format!("failed to list automation threads: {error}")),
        };
    let mut items = Vec::with_capacity(records.len());
    for record in &records {
        items.push(
            automation_thread_entry(&state, record, label.as_deref(), automation_deleted).await,
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "automationId": id,
            "automationLabel": label.unwrap_or_else(|| {
                records
                    .first()
                    .and_then(|record| record.automation_label_snapshot.clone())
                    .unwrap_or_else(|| id.clone())
            }),
            "automationDeleted": automation_deleted,
            "items": items,
            "count": items.len(),
            "total": total,
            "limit": limit,
            "offset": offset,
            "hasMore": offset + items.len() < total,
        })),
    )
}

pub async fn automation_activity(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<ActivityParams>,
) -> impl IntoResponse {
    let (service, job) = match automation_job(&state, &id).await {
        Ok(value) => value,
        Err(error) => return error,
    };
    let limit = params.limit.clamp(1, MAX_ACTIVITY_LIMIT);
    let runs = service.list_runs_for_job(&id, limit, params.offset).await;
    let target_thread_id = automation_target_thread(&job);
    let latest_thread_id = latest_automation_thread(runs.first()).or(target_thread_id.clone());
    let mut items = Vec::with_capacity(runs.len());
    for run in &runs {
        let thread_id = run
            .thread_id
            .clone()
            .or_else(|| target_thread_id.clone())
            .or_else(|| latest_thread_id.clone());
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
