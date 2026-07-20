//! Schedule math: cron expressions, timezones, and once-timestamps.

use chrono::{DateTime, Local, LocalResult, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use garyx_models::config::CronSchedule;
use std::str::FromStr;

/// Upper bound on interval schedules, in seconds (100 years). Kept far below
/// the point where `DateTime<Utc> + Duration` overflows chrono's representable
/// range, so an over-large interval is rejected with a clean error instead of
/// panicking in `compute_next_run`. No real automation cadence approaches this.
pub(super) const MAX_INTERVAL_SECS: u64 = 100 * 365 * 24 * 60 * 60;

pub(crate) fn parse_once_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(timestamp) = trimmed.parse::<DateTime<Utc>>() {
        return Some(timestamp);
    }

    if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Some(timestamp.with_timezone(&Utc));
    }

    let naive = trimmed
        .strip_prefix("ONCE:")
        .map(str::trim)
        .and_then(parse_local_once_naive)
        .or_else(|| parse_local_once_naive(trimmed))?;

    match Local.from_local_datetime(&naive) {
        LocalResult::Single(timestamp) => Some(timestamp.with_timezone(&Utc)),
        LocalResult::Ambiguous(first, _) => Some(first.with_timezone(&Utc)),
        LocalResult::None => None,
    }
}

pub(super) fn parse_local_once_naive(raw: &str) -> Option<NaiveDateTime> {
    for format in ["%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"] {
        if let Ok(timestamp) = NaiveDateTime::parse_from_str(raw, format) {
            return Some(timestamp);
        }
    }

    None
}

pub(super) fn parse_cron_schedule(expr: &str) -> Option<Schedule> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Primary format in Rust runtime: second-precision cron expression.
    if let Ok(schedule) = Schedule::from_str(trimmed) {
        return Some(schedule);
    }

    // Python parity: accept 5-field crontab expressions used by croniter.
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    if fields.len() == 5 {
        let normalized = format!("0 {trimmed}");
        if let Ok(schedule) = Schedule::from_str(&normalized) {
            return Some(schedule);
        }
    }

    None
}

/// Resolve the next cron firing after `start`, with the expression's fields
/// interpreted as wall-clock time in `tz`.
///
/// The cron crate's own timezone-aware iterator resolves every candidate via
/// `TimeZone::from_local_datetime(..).single()`, so on a DST fall-back day an
/// ambiguous wall-clock time (one that occurs twice) yields `None` and the
/// schedule silently skips the whole day. To match croniter semantics
/// instead, enumerate candidates on the naive wall clock (pretending it is
/// UTC so the cron crate performs no timezone resolution of its own), then
/// map each candidate back to a real instant: ambiguous times fire at their
/// earliest still-future occurrence, and times inside a spring-forward gap
/// skip to the next candidate.
///
/// Invariant: the returned instant is always `>= start`. Wall-clock
/// candidates are strictly after `start`'s wall clock, but across a
/// fall-back transition an ambiguous candidate's earlier instant can still
/// precede `start` in real time; returning it would arm `next_run` in the
/// past and storm-fire every scheduler tick until the transition passes.
pub(super) fn next_cron_run_in_timezone<Z: TimeZone>(
    schedule: &Schedule,
    start: DateTime<Utc>,
    tz: &Z,
) -> Option<DateTime<Utc>> {
    // Upper bound on consecutive gap-skipped candidates: a second-precision
    // expression has 3600 candidates inside a one-hour DST gap. Anything
    // beyond this bound means a pathological zone jump; returning `None`
    // there falls back to the hourly retry in `compute_next_run`, which
    // self-heals as `after` advances past the gap.
    const MAX_GAP_CANDIDATES: usize = 10_000;

    let start_wall = Utc.from_utc_datetime(&start.with_timezone(tz).naive_local());
    for candidate in schedule.after(&start_wall).take(MAX_GAP_CANDIDATES) {
        let (first, second) = match tz.from_local_datetime(&candidate.naive_utc()) {
            LocalResult::Single(instant) => (Some(instant), None),
            // Fall-back transition: the wall-clock time occurs twice; order
            // the pair by instant instead of trusting the tuple order —
            // `chrono::Local`'s platform-backed resolver has been observed
            // returning it swapped (review #TASK-1817), unlike chrono-tz.
            LocalResult::Ambiguous(a, b) => {
                if a <= b {
                    (Some(a), Some(b))
                } else {
                    (Some(b), Some(a))
                }
            }
            // Spring-forward gap: this wall-clock time never occurs.
            LocalResult::None => (None, None),
        };
        for instant in [first, second].into_iter().flatten() {
            let utc = instant.with_timezone(&Utc);
            if utc >= start {
                return Some(utc);
            }
        }
    }
    None
}

/// Resolve the timezone a bare cron expression (no explicit `timezone`)
/// should be interpreted in, as an IANA zone: the `TZ` environment variable
/// wins when it names a TZDB zone (legacy names like `EST5EDT` are real TZDB
/// zones and parse; full POSIX transition specs like `EST5EDT,M3.2.0/2`
/// fail to parse and fall through), otherwise the machine's configured
/// system zone.
///
/// Pure so tests can pin the precedence without touching process env.
pub(super) fn resolve_bare_cron_timezone(
    tz_env: Option<&str>,
    system_zone: Option<&str>,
) -> Option<Tz> {
    let parse = |value: Option<&str>| {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.parse::<Tz>().ok())
    };
    parse(tz_env).or_else(|| parse(system_zone))
}

/// [`resolve_bare_cron_timezone`] fed from the live process environment,
/// mirroring how `chrono::Local` itself honors `TZ` before the system zone.
pub(super) fn machine_cron_timezone() -> Option<Tz> {
    let tz_env = std::env::var("TZ").ok();
    let system_zone = iana_time_zone::get_timezone().ok();
    resolve_bare_cron_timezone(tz_env.as_deref(), system_zone.as_deref())
}

pub(super) fn has_non_empty_cron_text(value: Option<&str>) -> bool {
    value
        .map(str::trim)
        .is_some_and(|candidate| !candidate.is_empty())
}

pub(super) fn validate_cron_schedule(schedule: &CronSchedule) -> std::io::Result<()> {
    match schedule {
        CronSchedule::Interval { interval_secs } => {
            if *interval_secs > MAX_INTERVAL_SECS {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("interval schedule exceeds max interval_secs={MAX_INTERVAL_SECS}"),
                ));
            }
        }
        CronSchedule::Once { at } => {
            if parse_once_timestamp(at).is_none() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid once timestamp: {at}"),
                ));
            }
        }
        CronSchedule::Cron { expr, timezone } => {
            if parse_cron_schedule(expr).is_none() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid cron expression: {expr}"),
                ));
            }

            if let Some(tz_name) = timezone.as_deref().map(str::trim).filter(|s| !s.is_empty())
                && tz_name.parse::<Tz>().is_err()
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid cron timezone: {tz_name}"),
                ));
            }
        }
    }

    Ok(())
}
