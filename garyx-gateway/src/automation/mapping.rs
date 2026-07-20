//! Schedule mapping between the Automation product view
//! (`AutomationScheduleView`: Daily/Weekly/Monthly/Hours/Once) and the
//! engine's persisted `CronSchedule`.
//!
//! `compile_schedule` turns a product schedule into a cron/interval/once
//! schedule; `infer_schedule_view` reconstructs the product view from a
//! persisted schedule. The parsing/formatting helpers are private to this
//! translation.

use chrono::{DateTime, Local, Utc};
use garyx_models::config::{AutomationScheduleView, CronSchedule};

use super::http::trim_required;

/// Upper bound on interval schedules, in hours (100 years). Mirrors the cron
/// layer's `MAX_INTERVAL_SECS` so an over-large interval is rejected cleanly
/// here instead of overflowing chrono's `DateTime` math downstream.
pub(super) const MAX_INTERVAL_HOURS: u64 = 100 * 365 * 24;
pub(super) const WEEKDAY_CODES: [(&str, &str); 7] = [
    ("MON", "mo"),
    ("TUE", "tu"),
    ("WED", "we"),
    ("THU", "th"),
    ("FRI", "fr"),
    ("SAT", "sa"),
    ("SUN", "su"),
];

pub(super) fn parse_time_hm(raw: &str) -> Result<(u8, u8), String> {
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

pub(super) fn parse_month_day(day: u8) -> Result<u8, String> {
    if (1..=31).contains(&day) {
        Ok(day)
    } else {
        Err("schedule.day must be between 1 and 31".to_owned())
    }
}

pub(super) fn parse_once_input(raw: &str) -> Result<DateTime<Utc>, String> {
    super::engine::parse_once_timestamp(raw)
        .ok_or_else(|| "schedule.at must use YYYY-MM-DDTHH:MM or ONCE:YYYY-MM-DD HH:MM".to_owned())
}

pub(super) fn format_once_input(timestamp: DateTime<Utc>) -> String {
    timestamp
        .with_timezone(&Local)
        .format("%Y-%m-%dT%H:%M")
        .to_string()
}

pub(super) fn normalize_weekday(raw: &str) -> Option<&'static str> {
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

pub(super) fn weekday_short_code(raw: &str) -> Option<&'static str> {
    let normalized = raw.trim().to_ascii_uppercase();
    WEEKDAY_CODES
        .iter()
        .find_map(|(token, short)| (*token == normalized).then_some(*short))
}

pub(super) fn expand_weekday_expr(raw: &str) -> Result<Vec<String>, String> {
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
            let timestamp = super::engine::parse_once_timestamp(match schedule {
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
