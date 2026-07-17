//! Codex app-server agent provider.
//!
//! Rust port of `the original codex_provider.py`.
//! Implements `ProviderRuntime` backed by `codex_sdk::CodexClient`,
//! managing thread/turn lifecycle and streaming notifications.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use codex_sdk::types::{coerce_f64, coerce_i64};
use codex_sdk::{
    CodexClient, CodexClientConfig, CodexError, InputItem, JsonRpcNotification, ThreadForkParams,
    ThreadResumeParams, ThreadStartParams, TurnStartOptions,
};
use garyx_models::{
    is_builtin_provider_agent_id,
    provider::{
        CodexAppServerConfig, ImagePayload, PromptAttachment, ProviderMessage, ProviderMessageRole,
        ProviderRateLimit, ProviderRunOptions, ProviderRunResult, ProviderType, QueuedUserInput,
        SDK_SESSION_FORK_METADATA_KEY, SDK_SESSION_ID_METADATA_KEY, StreamBoundaryKind,
        StreamEvent, attachments_from_metadata, build_prompt_message_with_attachments,
    },
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::gary_prompt::{compose_gary_instructions, prepend_initial_context_to_user_message};
use crate::native_slash::build_native_skill_prompt;
use crate::provider_common::{
    PendingAckQueue, PendingRateLimits, garyx_mcp_server, metadata_bool, metadata_string,
    normalize_non_empty, resolve_run_id_with, runtime_env_overlay,
};
use crate::provider_trait::{
    BridgeError, ClearSessionOutcome, ProviderModelDefaults, ProviderRuntime,
    ProviderRuntimeSelection, StreamCallback,
};

const CODEX_CLIENT_IDLE_TTL: Duration = Duration::from_secs(180);
// `turn/steer` only acknowledges that follow-up input was queued into the
// active turn. It should not wait for the model response, but we keep this
// timeout wide enough for transient local load before falling back to replacing
// the stuck run.
const CODEX_STREAMING_INPUT_STEER_TIMEOUT: Duration = Duration::from_secs(30);
const CODEX_TIMEOUT_AUTO_CONTINUE_MESSAGE: &str = "continue";
const CODEX_TIMEOUT_AUTO_CONTINUE_METADATA_KEY: &str = "codex_timeout_auto_continue";

// ---------------------------------------------------------------------------
// Helper functions (provider-level domain mapping)
// ---------------------------------------------------------------------------

fn configured_request_timeout(seconds: f64) -> Duration {
    if seconds.is_finite() && seconds > 0.0 {
        Duration::from_secs_f64(seconds)
    } else {
        Duration::from_secs(300)
    }
}

fn normalize_thread_title(value: &str) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = normalized.trim();
    if trimmed.chars().count() <= 80 {
        return trimmed.to_owned();
    }
    let mut clipped = trimmed.chars().take(79).collect::<String>();
    clipped.push('…');
    clipped
}

fn extract_codex_thread_title(params: &Value) -> Option<String> {
    params
        .get("threadName")
        .or_else(|| params.get("thread_name"))
        .and_then(Value::as_str)
        .map(normalize_thread_title)
        .filter(|value| !value.is_empty())
}

fn extract_codex_thread_started_title(params: &Value) -> Option<String> {
    params
        .get("thread")
        .and_then(|thread| thread.get("name"))
        .and_then(Value::as_str)
        .map(normalize_thread_title)
        .filter(|value| !value.is_empty())
}

/// Check whether a notification's params match our expected thread/turn.
fn matches_turn(params: &Value, thread_id: &str, turn_id: &str) -> bool {
    if let Some(event_thread) = params.get("threadId").and_then(|v| v.as_str())
        && !event_thread.is_empty()
        && event_thread != thread_id
    {
        return false;
    }
    if let Some(event_turn) = params.get("turnId").and_then(|v| v.as_str())
        && !event_turn.is_empty()
        && event_turn != turn_id
    {
        return false;
    }
    if let Some(turn_obj_id) = params
        .get("turn")
        .and_then(|t| t.get("id"))
        .and_then(|v| v.as_str())
        && !turn_obj_id.is_empty()
        && turn_obj_id != turn_id
    {
        return false;
    }
    true
}

/// Whether Codex's `error.codexErrorInfo` classifier is a usage-limit
/// exhaustion. `CodexErrorInfo` is a oneOf: the relevant case is the bare
/// string `"usageLimitExceeded"`; tolerate an object-wrapped form too.
fn codex_error_is_usage_limit(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(code)) => code == "usageLimitExceeded",
        Some(Value::Object(map)) => map.contains_key("usageLimitExceeded"),
        _ => false,
    }
}

/// Extract the `RateLimitSnapshot` from an `account/rateLimits/updated`
/// notification. Per the app-server schema the snapshot lives under
/// `rateLimits`; tolerate a flattened shape (params *is* the snapshot) so a
/// future wire change does not silently disable quota detection.
fn extract_rate_limit_snapshot(params: &Value) -> Option<Value> {
    if let Some(snapshot) = params.get("rateLimits") {
        return Some(snapshot.clone());
    }
    if params.get("primary").is_some() || params.get("secondary").is_some() {
        return Some(params.clone());
    }
    None
}

/// One rolling rate-limit window from a Codex `RateLimitSnapshot`.
#[derive(Debug, Clone, Copy)]
struct CodexRateWindow {
    used_percent: i64,
    /// Unix seconds at which the window resets, when reported by Codex.
    resets_at: Option<i64>,
}

fn codex_rate_window(value: &Value) -> Option<CodexRateWindow> {
    let object = value.as_object()?;
    Some(CodexRateWindow {
        used_percent: object
            .get("usedPercent")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        resets_at: object.get("resetsAt").and_then(Value::as_i64),
    })
}

fn window_reset_key(window: &CodexRateWindow) -> i64 {
    window.resets_at.unwrap_or(i64::MIN)
}

/// Pick the binding rolling window: a saturated window blocks; when both are
/// saturated the one that resets latest is the real constraint; otherwise the
/// most-consumed window.
fn choose_blocking_window(
    primary: Option<CodexRateWindow>,
    secondary: Option<CodexRateWindow>,
) -> Option<(&'static str, CodexRateWindow)> {
    match (primary, secondary) {
        (Some(p), Some(s)) => {
            let p_saturated = p.used_percent >= 100;
            let s_saturated = s.used_percent >= 100;
            let pick = if p_saturated && s_saturated {
                if window_reset_key(&s) > window_reset_key(&p) {
                    ("secondary", s)
                } else {
                    ("primary", p)
                }
            } else if s_saturated {
                ("secondary", s)
            } else if p_saturated {
                ("primary", p)
            } else if s.used_percent > p.used_percent {
                ("secondary", s)
            } else {
                ("primary", p)
            };
            Some(pick)
        }
        (Some(p), None) => Some(("primary", p)),
        (None, Some(s)) => Some(("secondary", s)),
        (None, None) => None,
    }
}

fn unix_to_rfc3339(secs: i64) -> Option<String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0).map(|dt| dt.to_rfc3339())
}

/// Parse a concrete reset hint out of Codex's usage-limit message text.
///
/// Codex only attaches a structured `rateLimits` snapshot to some turns; when
/// it is absent, the human-readable error ("You've hit your usage limit. …
/// or try again at 9:42 PM.") is the only carrier of the reset time. Deriving
/// `reset_at` from it activates both the countdown banner and the gateway's
/// quota auto-resend, which key off `reset_at` presence.
///
/// Supported shapes (case-insensitive):
/// - "try again at 9:42 PM" / "try again at 10 pm" — a wall-clock time in the
///   gateway machine's local timezone (Codex CLI formats it locally). A time
///   more than five minutes in the past rolls over to tomorrow.
/// - "try again in 2 hours 13 minutes" / "try again in 45 minutes" — a
///   relative duration from `now`.
///
/// Returns RFC3339 UTC. `now` is injected so the parse is unit-testable.
fn reset_at_from_usage_message(
    message: &str,
    now: chrono::DateTime<chrono::Local>,
) -> Option<String> {
    reset_at_from_usage_message_in(message, now)
}

/// A relative "try again in …" hint longer than this is treated as garbage
/// from a malformed upstream message rather than a real quota window.
const MAX_MESSAGE_RESET_DAYS: i64 = 30;

/// Timezone-generic core of [`reset_at_from_usage_message`]; production passes
/// `chrono::Local`, tests pass a fixed `chrono_tz` zone so DST edges are
/// reproducible on any machine.
fn reset_at_from_usage_message_in<Tz: chrono::TimeZone>(
    message: &str,
    now: chrono::DateTime<Tz>,
) -> Option<String> {
    use chrono::{Duration as ChronoDuration, LocalResult, Utc};

    static AT_TIME: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)\btry again (?:at|after)\s+(\d{1,2})(?::(\d{2}))?\s*([ap])\.?m\b\.?",
        )
        .expect("valid regex")
    });
    static IN_DURATION: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)\btry again in\s+((?:\d+\s*(?:days?|hours?|minutes?|seconds?)\b(?:[,\s]|and\s)*)+)",
        )
        .expect("valid regex")
    });
    static DURATION_PART: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)(\d+)\s*(day|hour|minute|second)s?\b").expect("valid regex")
    });

    let zone = now.timezone();

    if let Some(captures) = AT_TIME.captures(message) {
        let hour12: u32 = captures.get(1)?.as_str().parse().ok()?;
        if !(1..=12).contains(&hour12) {
            return None;
        }
        let minute: u32 = captures
            .get(2)
            .map(|m| m.as_str().parse().ok())
            .unwrap_or(Some(0))?;
        if minute > 59 {
            return None;
        }
        let is_pm = captures.get(3)?.as_str().eq_ignore_ascii_case("p");
        let hour = match (hour12, is_pm) {
            (12, false) => 0,
            (12, true) => 12,
            (h, false) => h,
            (h, true) => h + 12,
        };

        // Resolve the wall-clock time in the given zone, tolerating DST edges:
        // an ambiguous time takes whichever occurrence is the earlier UTC
        // instant (chrono does not guarantee ordering of the Ambiguous pair),
        // a gap time slides an hour forward.
        let resolve = |naive: chrono::NaiveDateTime| match zone.from_local_datetime(&naive) {
            LocalResult::Single(dt) => Some(dt),
            LocalResult::Ambiguous(a, b) => Some(if a <= b { a } else { b }),
            LocalResult::None => zone
                .from_local_datetime(&(naive + ChronoDuration::hours(1)))
                .earliest(),
        };

        let today = now.date_naive().and_hms_opt(hour, minute, 0)?;
        let mut reset = resolve(today)?;
        // A reset time slightly in the past means the window just recovered;
        // keep it today so downstream consumers treat it as recovered instead
        // of scheduling a resend a full day out.
        if reset < now.clone() - ChronoDuration::minutes(5) {
            reset = resolve(today.checked_add_signed(ChronoDuration::days(1))?)?;
        }
        return Some(reset.with_timezone(&Utc).to_rfc3339());
    }

    if let Some(captures) = IN_DURATION.captures(message) {
        let body = captures.get(1)?.as_str();
        let mut total = ChronoDuration::zero();
        let mut matched = false;
        for part in DURATION_PART.captures_iter(body) {
            let amount: i64 = part.get(1)?.as_str().parse().ok()?;
            let unit = part.get(2)?.as_str().to_ascii_lowercase();
            // `try_*` + `checked_add` keep absurd amounts from a malformed
            // upstream message from panicking on Duration overflow.
            let step = match unit.as_str() {
                "day" => ChronoDuration::try_days(amount),
                "hour" => ChronoDuration::try_hours(amount),
                "minute" => ChronoDuration::try_minutes(amount),
                _ => ChronoDuration::try_seconds(amount),
            }?;
            total = total.checked_add(&step)?;
            matched = true;
        }
        if matched
            && total > ChronoDuration::zero()
            && total <= ChronoDuration::days(MAX_MESSAGE_RESET_DAYS)
        {
            return Some(
                now.checked_add_signed(total)?
                    .with_timezone(&Utc)
                    .to_rfc3339(),
            );
        }
    }

    None
}

/// Build a `ProviderRateLimit` from Codex's structured quota signal. Returns
/// `None` unless Codex actually reported a usage-limit error, a
/// `rateLimitReachedType`, or a saturated window — so it is safe to call on
/// every failed run.
fn build_codex_rate_limit(
    provider_slug: &str,
    usage_limit_hit: bool,
    snapshot: Option<&Value>,
    message: Option<&str>,
) -> Option<ProviderRateLimit> {
    let snapshot_obj = snapshot.and_then(Value::as_object);
    let reached_type = snapshot_obj
        .and_then(|object| object.get("rateLimitReachedType"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let primary = snapshot_obj
        .and_then(|object| object.get("primary"))
        .and_then(codex_rate_window);
    let secondary = snapshot_obj
        .and_then(|object| object.get("secondary"))
        .and_then(codex_rate_window);
    let blocking = choose_blocking_window(primary, secondary);

    // Classify as rate-limited only on an explicit quota signal: the current
    // turn's `usageLimitExceeded` error, or a snapshot `rateLimitReachedType`
    // (the schema only sets that when a limit was actually reached). A merely
    // saturated window (`usedPercent >= 100`) without either signal is NOT
    // enough — an account can sit at 100% on one window while a run fails for an
    // unrelated reason, and misreading that as a quota exhaustion would resend
    // the user's message spuriously. Saturation is still used to pick which
    // window's reset time to report, just not to trigger.
    if !usage_limit_hit && reached_type.is_none() {
        return None;
    }

    let (window_label, window) = match blocking {
        Some((label, window)) => (Some(label.to_owned()), Some(window)),
        None => (None, None),
    };

    // Prefer the structured snapshot's reset; fall back to the reset hint
    // embedded in the usage-limit message ("… try again at 9:42 PM"), which is
    // the only carrier when Codex omits the snapshot. `reset_at` presence is
    // what downstream turns into the countdown and the quota auto-resend.
    let reset_at = window
        .and_then(|window| window.resets_at)
        .and_then(unix_to_rfc3339)
        .or_else(|| {
            message.and_then(|message| reset_at_from_usage_message(message, chrono::Local::now()))
        });

    Some(ProviderRateLimit {
        provider: provider_slug.to_owned(),
        reset_at,
        window: window_label,
        used_percent: window.map(|window| window.used_percent),
        reached_type,
        message: message
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
    })
}

/// Extract usage (input_tokens, output_tokens, cost) from a completed turn.
fn extract_usage(turn: &Value) -> (i64, i64, f64) {
    let usage = match turn.get("usage") {
        Some(u) if u.is_object() => u,
        _ => return (0, 0, 0.0),
    };

    let input_tokens = ["inputTokens", "input_tokens", "input", "prompt_tokens"]
        .iter()
        .find_map(|k| usage.get(*k).filter(|v| !v.is_null()))
        .map(coerce_i64)
        .unwrap_or(0);

    let output_tokens = [
        "outputTokens",
        "output_tokens",
        "output",
        "completion_tokens",
    ]
    .iter()
    .find_map(|k| usage.get(*k).filter(|v| !v.is_null()))
    .map(coerce_i64)
    .unwrap_or(0);

    let cost = ["totalCostUsd", "total_cost_usd", "costUsd", "cost"]
        .iter()
        .find_map(|k| usage.get(*k).filter(|v| !v.is_null()))
        .map(coerce_f64)
        .unwrap_or(0.0);

    (input_tokens, output_tokens, cost)
}

/// Build typed `InputItem` vector from `ProviderRunOptions`.
fn build_input_items_from_parts(
    message: &str,
    images: &[ImagePayload],
    attachments: &[PromptAttachment],
) -> Vec<InputItem> {
    let message = build_prompt_message_with_attachments(message, attachments);
    if !attachments.is_empty() {
        return vec![InputItem::Text { text: message }];
    }

    let mut items = Vec::with_capacity(images.len() + 1);
    if !message.trim().is_empty() || images.is_empty() {
        items.push(InputItem::Text { text: message });
    }

    for image in images {
        if image.data.trim().is_empty() {
            continue;
        }
        items.push(InputItem::Image {
            url: format!("data:{};base64,{}", image.media_type, image.data),
        });
    }

    items
}

#[derive(Debug, Default, serde::Deserialize)]
struct CodexCliConfigFile {
    model: Option<String>,
}

fn is_custom_standalone_agent(metadata: &HashMap<String, Value>) -> bool {
    metadata
        .get("agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some_and(|value| !is_builtin_provider_agent_id(value))
}

fn default_codex_config_path() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CODEX_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(home).join("config.toml"));
    }

    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".codex").join("config.toml"))
}

fn read_codex_cli_default_model_from_path(path: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    let parsed: CodexCliConfigFile = toml::from_str(&contents).ok()?;
    normalize_non_empty(parsed.model.as_deref())
}

/// Human-readable message for account/config-scoped advisory notifications
/// (`warning`, `configWarning`, `deprecationNotice`) that carry no turn
/// affinity. Returns `None` for every other method.
fn codex_advisory_notification_message(method: &str, params: &Value) -> Option<String> {
    let text_field = |key: &str| {
        params
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    };
    let with_details = |summary: String| match text_field("details") {
        Some(details) => format!("{summary} ({details})"),
        None => summary,
    };
    match method {
        "warning" => text_field("message"),
        "configWarning" => text_field("summary").map(|summary| {
            let summary = with_details(summary);
            match text_field("path") {
                Some(path) => format!("{summary} [config: {path}]"),
                None => summary,
            }
        }),
        "deprecationNotice" => text_field("summary").map(with_details),
        _ => None,
    }
}

/// Extract the destination model from a `model/rerouted` notification.
fn extract_rerouted_model(params: &Value) -> Option<String> {
    params
        .get("toModel")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

/// Cumulative `(input, output)` totals carried by a `thread/tokenUsage/updated`
/// snapshot's `total` breakdown.
fn token_usage_totals(snapshot: &Value) -> (i64, i64) {
    let read = |key: &str| {
        snapshot
            .get("total")
            .and_then(|total| total.get(key))
            .map(coerce_i64)
            .unwrap_or(0)
    };
    (read("inputTokens"), read("outputTokens"))
}

/// Per-turn usage bookkeeping over `thread/tokenUsage/updated` notifications.
///
/// As of app-server 0.144 `turn/completed` no longer carries usage, so a
/// turn's consumption must derive from these snapshots. `total` is
/// thread-cumulative, so the turn's own usage is the growth of `total` over a
/// pre-turn baseline. The baseline comes from snapshots that carry a *prior*
/// turn id (resume/fork replay emits one), or from the totals remembered when
/// the previous in-process turn on the same thread finished. Deriving from
/// `total` growth alone also keeps a stale snapshot that Codex re-sends under
/// the current turn id (e.g. an immediate usage-limit failure) at zero, since
/// its totals have not moved.
#[derive(Default)]
struct CodexTurnUsageTracker {
    replay_baseline: Option<(i64, i64)>,
    first: Option<Value>,
    latest: Option<Value>,
}

impl CodexTurnUsageTracker {
    /// Feed a `thread/tokenUsage/updated` notification. Returns `true` when
    /// the notification belonged to this tracker's thread and was consumed.
    fn observe(&mut self, params: &Value, thread_id: &str, turn_id: &str) -> bool {
        let Some(usage) = params.get("tokenUsage").filter(|v| v.is_object()) else {
            return false;
        };
        let event_thread = params.get("threadId").and_then(Value::as_str);
        if event_thread != Some(thread_id) {
            // Codex clients are shared across Garyx threads; another thread's
            // usage must influence neither this turn nor its baseline.
            return false;
        }
        let event_turn = params.get("turnId").and_then(Value::as_str);
        if event_turn == Some(turn_id) {
            if self.first.is_none() {
                self.first = Some(usage.clone());
            }
            self.latest = Some(usage.clone());
        } else {
            // A snapshot for a prior turn (resume/fork replay): the thread's
            // cumulative totals immediately before this turn.
            let totals = token_usage_totals(usage);
            self.replay_baseline = Some(max_totals(self.replay_baseline, totals));
        }
        true
    }

    /// The turn's `(input_tokens, output_tokens)`.
    ///
    /// `stored_baseline` is the totals remembered from the previous in-process
    /// turn on this thread; `thread_was_resumed` reports whether this run
    /// attached to an existing Codex thread (resume or fork).
    fn finish(&self, stored_baseline: Option<(i64, i64)>, thread_was_resumed: bool) -> (i64, i64) {
        let Some(latest) = self.latest.as_ref() else {
            return (0, 0);
        };
        let latest_totals = token_usage_totals(latest);
        let baseline = match (self.replay_baseline, stored_baseline) {
            (Some(replay), Some(stored)) => Some(max_totals(Some(replay), stored)),
            (replay, stored) => replay.or(stored),
        };
        match baseline {
            Some((base_in, base_out)) => (
                (latest_totals.0 - base_in).max(0),
                (latest_totals.1 - base_out).max(0),
            ),
            // Fresh thread: totals started from zero, so they are the turn.
            None if !thread_was_resumed => latest_totals,
            // Resumed thread with no observable baseline: best effort from the
            // first current-turn snapshot's last-request breakdown plus the
            // `total` growth observed since.
            None => {
                let Some(first) = self.first.as_ref() else {
                    return (0, 0);
                };
                let first_totals = token_usage_totals(first);
                let read_last = |snapshot: &Value, key: &str| {
                    snapshot
                        .get("last")
                        .and_then(|last| last.get(key))
                        .map(coerce_i64)
                        .unwrap_or(0)
                };
                (
                    read_last(first, "inputTokens") + (latest_totals.0 - first_totals.0).max(0),
                    read_last(first, "outputTokens") + (latest_totals.1 - first_totals.1).max(0),
                )
            }
        }
    }

    /// The latest cumulative totals observed for this thread, to remember as
    /// the next turn's baseline.
    fn latest_totals(&self) -> Option<(i64, i64)> {
        let observed = self
            .latest
            .as_ref()
            .map(token_usage_totals)
            .map(|totals| max_totals(self.replay_baseline, totals));
        observed.or(self.replay_baseline)
    }
}

/// Element-wise max of monotonic cumulative totals.
fn max_totals(current: Option<(i64, i64)>, candidate: (i64, i64)) -> (i64, i64) {
    match current {
        Some((input, output)) => (input.max(candidate.0), output.max(candidate.1)),
        None => candidate,
    }
}

fn resolve_codex_actual_model_with_config_path(
    config: &CodexAppServerConfig,
    metadata: &HashMap<String, Value>,
    config_path: Option<&Path>,
) -> Option<String> {
    normalize_non_empty(metadata.get("model").and_then(Value::as_str))
        .or_else(|| normalize_non_empty(Some(config.model.as_str())))
        .or_else(|| normalize_non_empty(Some(config.default_model.as_str())))
        .or_else(|| config_path.and_then(read_codex_cli_default_model_from_path))
}

fn resolve_codex_actual_model(
    config: &CodexAppServerConfig,
    metadata: &HashMap<String, Value>,
) -> Option<String> {
    // Only fall back to `~/.codex/config.toml` for the real Codex provider.
    // Traex stores its config under `~/.trae`, so reading Codex's config here
    // would leak Codex's default model into Traex run metadata/logs.
    let config_path = if config.provider_type == ProviderType::CodexAppServer {
        default_codex_config_path()
    } else {
        None
    };
    resolve_codex_actual_model_with_config_path(config, metadata, config_path.as_deref())
}

fn normalize_codex_mcp_servers(metadata: &HashMap<String, Value>) -> Option<Value> {
    let servers = metadata.get("remote_mcp_servers")?.as_object()?;
    let mut normalized = serde_json::Map::new();

    for (name, raw_server) in servers {
        let Some(server) = raw_server.as_object() else {
            continue;
        };
        let mut entry = serde_json::Map::new();

        if let Some(command) = server
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            entry.insert("command".to_owned(), Value::String(command.to_owned()));
            entry.insert(
                "args".to_owned(),
                Value::Array(
                    server
                        .get("args")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|item| item.as_str().map(|value| value.to_owned()))
                                .map(Value::String)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default(),
                ),
            );

            let env = server
                .get("env")
                .and_then(Value::as_object)
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|(env_key, env_value)| {
                            env_value.as_str().map(|env_value| {
                                (env_key.clone(), Value::String(env_value.to_owned()))
                            })
                        })
                        .collect::<serde_json::Map<_, _>>()
                })
                .unwrap_or_default();
            if !env.is_empty() {
                entry.insert("env".to_owned(), Value::Object(env));
            }
            if let Some(enabled) = server.get("enabled").and_then(Value::as_bool) {
                entry.insert("enabled".to_owned(), Value::Bool(enabled));
            }
            if let Some(cwd) = server
                .get("cwd")
                .or_else(|| server.get("working_dir"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let canonical = std::fs::canonicalize(cwd)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| cwd.to_owned());
                entry.insert("cwd".to_owned(), Value::String(canonical));
            }
        } else if let Some(url) = server
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            entry.insert("url".to_owned(), Value::String(url.to_owned()));
            if let Some(enabled) = server.get("enabled").and_then(Value::as_bool) {
                entry.insert("enabled".to_owned(), Value::Bool(enabled));
            }
            if let Some(headers) = server.get("headers").and_then(Value::as_object) {
                let http_headers = headers
                    .iter()
                    .filter_map(|(header_key, header_value)| {
                        header_value.as_str().map(|header_value| {
                            (header_key.clone(), Value::String(header_value.to_owned()))
                        })
                    })
                    .collect::<serde_json::Map<_, _>>();
                if !http_headers.is_empty() {
                    entry.insert("http_headers".to_owned(), Value::Object(http_headers));
                }
            }
            if matches!(
                server.get("type").and_then(Value::as_str),
                Some(kind) if kind.eq_ignore_ascii_case("sse")
            ) {
                entry.insert("transport".to_owned(), Value::String("sse".to_owned()));
            }
        }

        if !entry.is_empty() {
            normalized.insert(name.clone(), Value::Object(entry));
        }
    }

    (!normalized.is_empty()).then_some(Value::Object(normalized))
}

fn resolve_runtime_codex_env(
    config: &CodexAppServerConfig,
    metadata: &HashMap<String, Value>,
) -> HashMap<String, String> {
    runtime_env_overlay(&config.env, metadata, "provider_env")
}

fn build_codex_thread_config(
    provider_config: &CodexAppServerConfig,
    metadata: &HashMap<String, Value>,
    thread_id: &str,
    run_id: &str,
) -> Option<Value> {
    let mut thread_config = serde_json::Map::new();

    let runtime_instructions = metadata_string(metadata, "developer_instructions")
        .or_else(|| metadata_string(metadata, "system_prompt"));
    if runtime_instructions.is_some() || !is_custom_standalone_agent(metadata) {
        let instructions = compose_gary_instructions(runtime_instructions.as_deref());
        thread_config.insert(
            "developer_instructions".to_owned(),
            Value::String(instructions),
        );
    }

    let mut mcp_servers = match normalize_codex_mcp_servers(metadata) {
        Some(Value::Object(obj)) => obj,
        _ => serde_json::Map::new(),
    };
    if let Some(server) =
        garyx_mcp_server(&provider_config.mcp_base_url, thread_id, run_id, metadata)
    {
        // Reserve `garyx` for the built-in local gateway endpoint so runtime
        // metadata cannot shadow it with a stale or malformed URL.
        mcp_servers.insert(
            "garyx".to_owned(),
            json!({
                "url": server.url,
                "http_headers": server.headers,
            }),
        );
    }
    if !mcp_servers.is_empty() {
        thread_config.insert("mcp_servers".to_owned(), Value::Object(mcp_servers));
    }
    (!thread_config.is_empty()).then_some(Value::Object(thread_config))
}

fn build_input_items(options: &ProviderRunOptions, include_memory: bool) -> Vec<InputItem> {
    let message = build_native_skill_prompt(&options.message, &options.metadata)
        .unwrap_or_else(|| options.message.clone());
    let message =
        prepend_initial_context_to_user_message(&message, &options.metadata, include_memory);
    let attachments = attachments_from_metadata(&options.metadata);
    build_input_items_from_parts(
        &message,
        options.images.as_deref().unwrap_or_default(),
        &attachments,
    )
}

fn resolve_codex_request_model(
    config: &CodexAppServerConfig,
    metadata: &HashMap<String, Value>,
) -> Option<String> {
    metadata_string(metadata, "model").or_else(|| {
        if !config.model.trim().is_empty() {
            Some(config.model.clone())
        } else if !config.default_model.trim().is_empty() {
            Some(config.default_model.clone())
        } else {
            None
        }
    })
}

fn resolve_codex_request_reasoning_effort(
    config: &CodexAppServerConfig,
    metadata: &HashMap<String, Value>,
) -> Option<String> {
    metadata_string(metadata, "model_reasoning_effort").or_else(|| {
        (!config.model_reasoning_effort.trim().is_empty())
            .then(|| config.model_reasoning_effort.clone())
    })
}

fn resolve_codex_request_service_tier(
    config: &CodexAppServerConfig,
    metadata: &HashMap<String, Value>,
) -> Option<String> {
    metadata_string(metadata, "model_service_tier").or_else(|| {
        (!config.model_service_tier.trim().is_empty()).then(|| config.model_service_tier.clone())
    })
}

fn build_turn_start_options(
    config: &CodexAppServerConfig,
    metadata: &HashMap<String, Value>,
) -> TurnStartOptions {
    TurnStartOptions {
        model: resolve_codex_request_model(config, metadata),
        effort: resolve_codex_request_reasoning_effort(config, metadata),
        service_tier: resolve_codex_request_service_tier(config, metadata),
    }
}

fn append_codex_assistant_session_message(
    session_messages: &mut Vec<ProviderMessage>,
    item_id: Option<&str>,
    delta: &str,
) {
    if delta.is_empty() {
        return;
    }

    let normalized_item_id = item_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let can_append = session_messages.last().is_some_and(|message| {
        message.role == ProviderMessageRole::Assistant
            && message.metadata.get("source").and_then(Value::as_str) == Some("codex_app_server")
            && message
                .metadata
                .get("item_id")
                .and_then(Value::as_str)
                .map(|value| value.to_owned())
                == normalized_item_id
    });

    if can_append {
        if let Some(last) = session_messages.last_mut() {
            let mut text = last.text.clone().unwrap_or_default();
            text.push_str(delta);
            last.text = Some(text.clone());
            last.content = Value::String(text);
        }
        return;
    }

    let mut message = ProviderMessage::assistant_text(delta)
        .with_timestamp(chrono::Utc::now().to_rfc3339())
        .with_metadata_value("source", serde_json::json!("codex_app_server"))
        .with_metadata_value("item_type", serde_json::json!("agentMessage"));
    if let Some(item_id) = normalized_item_id {
        message = message.with_metadata_value("item_id", serde_json::json!(item_id));
    }
    session_messages.push(message);
}

/// Build a tool session message from an item notification.
fn build_tool_session_message(item: &Value, is_completed: bool) -> Option<ProviderMessage> {
    let item_type = codex_thread_item_type(item)?;
    if !is_codex_structured_activity_item_type(item_type) {
        return None;
    }

    let tool_use_id = item
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();

    let tool_name = codex_structured_activity_name(item_type, item);

    // Garyx's existing stream protocol represents provider-side structured
    // activity as ToolUse/ToolResult frames. Preserve Codex's original item
    // type in metadata so each channel can decide how to render it.
    let mut msg = if is_completed {
        ProviderMessage::tool_result(
            item.clone(),
            (!tool_use_id.is_empty()).then_some(tool_use_id),
            Some(tool_name),
            Some(codex_structured_activity_is_error(item)),
        )
    } else {
        ProviderMessage::tool_use(
            item.clone(),
            (!tool_use_id.is_empty()).then_some(tool_use_id),
            Some(tool_name),
        )
    };

    msg = msg
        .with_timestamp(chrono::Utc::now().to_rfc3339())
        .with_metadata_value("source", serde_json::json!("codex_app_server"))
        .with_metadata_value("item_type", serde_json::json!(item_type));

    Some(msg)
}

fn codex_thread_item_type(item: &Value) -> Option<&str> {
    item.get("type")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
}

/// Record a started structured-activity item so its completion can be paired.
fn tool_session_message_for_started_item(
    item: &Value,
    started_item_ids: &mut HashSet<String>,
) -> Option<ProviderMessage> {
    let msg = build_tool_session_message(item, false)?;
    if let Some(id) = msg.tool_use_id.as_deref().filter(|id| !id.is_empty()) {
        started_item_ids.insert(id.to_owned());
    }
    Some(msg)
}

/// Map a completed structured-activity item to session messages.
///
/// Some item types are completed-only on the wire (app-server 0.144 emits
/// `subAgentActivity` solely as `item/completed`). A lone `ToolResult` frame is
/// invisible on channels that render tool activity from the `ToolUse` frame,
/// so when no matching `item/started` was seen, synthesize the paired
/// `ToolUse` first. This stays inside the provider-neutral stream contract.
fn tool_session_messages_for_completed_item(
    item: &Value,
    started_item_ids: &mut HashSet<String>,
) -> Vec<ProviderMessage> {
    let Some(completed) = build_tool_session_message(item, true) else {
        return Vec::new();
    };
    let mut messages = Vec::with_capacity(2);
    if let Some(id) = completed.tool_use_id.as_deref().filter(|id| !id.is_empty())
        && !started_item_ids.contains(id)
    {
        // Mark as started so a duplicate completion cannot re-synthesize.
        started_item_ids.insert(id.to_owned());
        if let Some(started) = build_tool_session_message(item, false) {
            messages.push(started);
        }
    }
    messages.push(completed);
    messages
}

fn is_codex_structured_activity_item_type(item_type: &str) -> bool {
    // "reasoning" is intentionally excluded: Codex internal chain-of-thought
    // must not be persisted as Garyx activity (#TASK-963).
    [
        "hookPrompt",
        "plan",
        "commandExecution",
        "fileChange",
        "mcpToolCall",
        "dynamicToolCall",
        "collabAgentToolCall",
        "subAgentActivity",
        "webSearch",
        "imageView",
        "imageGeneration",
        "sleep",
        "enteredReviewMode",
        "exitedReviewMode",
        "contextCompaction",
    ]
    .iter()
    .any(|candidate| item_type.eq_ignore_ascii_case(candidate))
}

fn codex_structured_activity_name(item_type: &str, item: &Value) -> String {
    if item_type.eq_ignore_ascii_case("mcpToolCall") {
        let server = item.get("server").and_then(|v| v.as_str()).unwrap_or("");
        let tool = item.get("tool").and_then(|v| v.as_str()).unwrap_or("");
        if !tool.is_empty() {
            return format!("mcp:{server}:{tool}");
        }
    }

    if item_type.eq_ignore_ascii_case("dynamicToolCall") {
        let namespace = item
            .get("namespace")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let tool = item
            .get("tool")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        return match (namespace, tool) {
            (Some(namespace), Some(tool)) => format!("{namespace}:{tool}"),
            (_, Some(tool)) => tool.to_owned(),
            _ => item_type.to_owned(),
        };
    }

    if item_type.eq_ignore_ascii_case("collabAgentToolCall")
        && let Some(tool) = item
            .get("tool")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
    {
        return tool.to_owned();
    }

    // Sub-agent delegation activity (Codex multi-agent v2, e.g. the GPT-5.6
    // `ultra` reasoning tier). Name it by the delegated agent path so channels
    // and transcript rows show which sub-agent is active.
    if item_type.eq_ignore_ascii_case("subAgentActivity") {
        if let Some(agent_path) = item
            .get("agentPath")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return format!("subAgent:{agent_path}");
        }
        return "subAgent".to_owned();
    }

    item_type.to_owned()
}

fn codex_structured_activity_is_error(item: &Value) -> bool {
    let failed_status = item
        .get("status")
        .and_then(|v| v.as_str())
        .map(|status| {
            let status = status.trim();
            status.eq_ignore_ascii_case("failed")
                || status.eq_ignore_ascii_case("declined")
                || status.eq_ignore_ascii_case("error")
                || status.eq_ignore_ascii_case("canceled")
                || status.eq_ignore_ascii_case("cancelled")
        })
        .unwrap_or(false);
    if failed_status {
        return true;
    }

    let explicit_failure = item
        .get("success")
        .and_then(Value::as_bool)
        .map(|success| !success)
        .unwrap_or(false);
    explicit_failure || item.get("error").is_some_and(|error| !error.is_null())
}

fn is_agent_message_item(item: &Value) -> bool {
    item.get("type")
        .and_then(|v| v.as_str())
        .map(|kind| kind.eq_ignore_ascii_case("agentMessage"))
        .unwrap_or(false)
}

fn is_user_message_item(item: &Value) -> bool {
    item.get("type")
        .and_then(|v| v.as_str())
        .map(|kind| kind.eq_ignore_ascii_case("userMessage"))
        .unwrap_or(false)
}

#[cfg(test)]
fn is_tool_activity_item(item: &Value) -> bool {
    codex_thread_item_type(item)
        .map(is_codex_structured_activity_item_type)
        .unwrap_or(false)
}

fn maybe_emit_agent_message_separator(
    next_item_id: Option<&str>,
    current_item_id: &mut Option<String>,
    current_item_has_text: &mut bool,
    response_parts: &mut Vec<String>,
    on_chunk: &(dyn Fn(StreamEvent) + Send + Sync),
) {
    let Some(next_item_id) = next_item_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return;
    };

    let switched_items = current_item_id
        .as_deref()
        .map(|current| current != next_item_id)
        .unwrap_or(false);

    if switched_items && *current_item_has_text {
        let separator = "\n\n".to_owned();
        response_parts.push(separator.clone());
        on_chunk(StreamEvent::Boundary {
            kind: StreamBoundaryKind::AssistantSegment,
            pending_input_id: None,
        });
    }

    if current_item_id.as_deref() != Some(next_item_id) {
        *current_item_id = Some(next_item_id.to_owned());
        *current_item_has_text = false;
    }
}

fn emit_tool_stream_event(
    message: &ProviderMessage,
    on_chunk: &(dyn Fn(StreamEvent) + Send + Sync),
) {
    match message.role_str() {
        "tool_use" => on_chunk(StreamEvent::ToolUse {
            message: message.clone(),
        }),
        "tool_result" => on_chunk(StreamEvent::ToolResult {
            message: message.clone(),
        }),
        _ => {}
    }
}

/// Build `ThreadStartParams` from `CodexAppServerConfig`.
fn build_thread_start_params(
    config: &CodexAppServerConfig,
    workspace_dir_override: Option<&str>,
    thread_id: &str,
    run_id: &str,
    metadata: &HashMap<String, Value>,
) -> ThreadStartParams {
    let cwd = workspace_dir_override
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            config
                .workspace_dir
                .as_ref()
                .filter(|d| !d.is_empty())
                .cloned()
        });
    let model = resolve_codex_request_model(config, metadata);
    let model_reasoning_effort = resolve_codex_request_reasoning_effort(config, metadata);
    let service_tier = resolve_codex_request_service_tier(config, metadata);

    ThreadStartParams {
        cwd: cwd.clone(),
        config: build_codex_thread_config(config, metadata, thread_id, run_id),
        model,
        model_reasoning_effort,
        service_tier,
        approval_policy: if config.approval_policy.is_empty() {
            None
        } else {
            Some(config.approval_policy.clone())
        },
        sandbox: if config.sandbox_mode.is_empty() {
            None
        } else {
            Some(config.sandbox_mode.clone())
        },
    }
}

/// Map a `CodexError` into a `BridgeError`.
fn map_codex_error(context: &str, e: CodexError) -> BridgeError {
    BridgeError::RunFailed(format!("{context}: {e}"))
}

fn resolve_existing_thread_id(
    session_map: &HashMap<String, String>,
    thread_id: &str,
    sdk_session_id: Option<&str>,
) -> Option<String> {
    session_map
        .get(thread_id)
        .cloned()
        .or_else(|| sdk_session_id.map(ToOwned::to_owned))
}

fn thread_fork_params_from_start(
    parent_thread_id: String,
    thread_params: &ThreadStartParams,
) -> ThreadForkParams {
    ThreadForkParams {
        thread_id: parent_thread_id,
        cwd: thread_params.cwd.clone(),
        config: thread_params.config.clone(),
        model: thread_params.model.clone(),
        model_reasoning_effort: thread_params.model_reasoning_effort.clone(),
        service_tier: thread_params.service_tier.clone(),
        approval_policy: thread_params.approval_policy.clone(),
        sandbox: thread_params.sandbox.clone(),
    }
}

async fn resume_or_start_thread<Resume, ResumeFut, Fork, ForkFut, Start, StartFut>(
    existing_thread_id: Option<String>,
    fork_session: bool,
    thread_params: ThreadStartParams,
    mut resume: Resume,
    mut fork: Fork,
    mut start: Start,
) -> Result<String, BridgeError>
where
    Resume: FnMut(ThreadResumeParams) -> ResumeFut,
    ResumeFut: Future<Output = Result<String, CodexError>>,
    Fork: FnMut(ThreadForkParams) -> ForkFut,
    ForkFut: Future<Output = Result<String, CodexError>>,
    Start: FnMut(ThreadStartParams) -> StartFut,
    StartFut: Future<Output = Result<String, CodexError>>,
{
    if let Some(existing_thread_id) = existing_thread_id {
        if fork_session {
            let fork_params =
                thread_fork_params_from_start(existing_thread_id.clone(), &thread_params);
            return fork(fork_params)
                .await
                .map_err(|e| map_codex_error("thread/fork failed", e));
        }

        let resume_params = ThreadResumeParams {
            thread_id: existing_thread_id.clone(),
            cwd: thread_params.cwd.clone(),
            config: thread_params.config.clone(),
            model: thread_params.model.clone(),
            model_reasoning_effort: thread_params.model_reasoning_effort.clone(),
            service_tier: thread_params.service_tier.clone(),
            approval_policy: thread_params.approval_policy.clone(),
            sandbox: thread_params.sandbox.clone(),
        };

        match resume(resume_params).await {
            Ok(thread_id) => return Ok(thread_id),
            Err(error) => {
                tracing::warn!(
                    thread_id = %existing_thread_id,
                    error = %error,
                    "codex resume failed, starting new thread"
                );
            }
        }
    }

    if fork_session {
        return Err(BridgeError::SessionError(
            "codex fork requested without parent thread id".to_owned(),
        ));
    }

    start(thread_params)
        .await
        .map_err(|e| map_codex_error("thread/start failed", e))
}

// ---------------------------------------------------------------------------
// CodexAgentProvider
// ---------------------------------------------------------------------------

/// Agent provider backed by `codex app-server` via `codex_sdk::CodexClient`.
pub struct CodexAgentProvider {
    config: CodexAppServerConfig,
    /// Hot-reloadable model defaults. Config reloads reconcile onto the live
    /// provider instance (the provider key excludes model defaults to keep
    /// thread affinity stable), so default-model resolution must read these
    /// instead of the frozen `config` fields.
    model_defaults: std::sync::RwLock<ProviderModelDefaults>,
    clients: CodexClientMap,
    /// Maps Garyx thread IDs to codex thread IDs.
    session_map: Mutex<HashMap<String, String>>,
    /// run_id -> active Codex thread/turn record.
    active_runs: Mutex<HashMap<String, ActiveCodexRun>>,
    /// thread_id -> (codex_thread_id, turn_id, run_id)
    active_session_turns: Mutex<HashMap<String, (String, String, String)>>,
    /// thread_id -> (run_id, live callback)
    active_session_callbacks: Mutex<HashMap<String, ActiveSessionCallback>>,
    /// thread_id -> (run_id, pending userMessage markers waiting for Codex item events)
    active_session_pending_acks: Mutex<HashMap<String, PendingCodexAcks>>,
    /// thread_id -> rate-limit context captured when the last run terminated
    /// because the ChatGPT-plan usage quota was exhausted. Consumed once by the
    /// bridge run-completion path via `take_rate_limit`.
    pending_rate_limits: PendingRateLimits,
    /// codex_thread_id -> last observed cumulative `(input, output)` token
    /// totals from `thread/tokenUsage/updated`. Serves as the usage baseline
    /// for the next turn on the same in-process thread; resumed threads get
    /// their baseline from the replayed prior-turn snapshot instead.
    thread_usage_totals: Mutex<HashMap<String, (i64, i64)>>,
    ready: Mutex<bool>,
}

type CodexClientMap = Arc<Mutex<HashMap<String, Arc<CodexClientSlot>>>>;
type ActiveSessionCallback = (String, Arc<dyn Fn(StreamEvent) + Send + Sync>);
type PendingCodexAcks = (String, PendingAckQueue);

struct CodexClientSlot {
    client: Mutex<CodexClient>,
    env: HashMap<String, String>,
    active_runs: AtomicUsize,
    last_used: Mutex<Instant>,
}

#[derive(Debug, Clone)]
struct ActiveCodexRun {
    garyx_thread_id: String,
    codex_thread_id: String,
    turn_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexClientReuseDecision {
    Reuse,
    ReplaceIdle,
}

fn decide_codex_client_reuse(
    existing_env: &HashMap<String, String>,
    desired_env: &HashMap<String, String>,
    active_run_count: usize,
) -> CodexClientReuseDecision {
    if existing_env == desired_env || active_run_count > 0 {
        CodexClientReuseDecision::Reuse
    } else {
        CodexClientReuseDecision::ReplaceIdle
    }
}

fn codex_run_result_timed_out(result: &Result<ProviderRunResult, BridgeError>) -> bool {
    match result {
        Ok(result) => {
            !result.success
                && result
                    .error
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|value| value.eq_ignore_ascii_case("timeout"))
        }
        Err(BridgeError::Timeout) => true,
        Err(_) => false,
    }
}

fn codex_timeout_auto_continue_options(options: &ProviderRunOptions) -> ProviderRunOptions {
    let mut options = options.clone();
    options.message = CODEX_TIMEOUT_AUTO_CONTINUE_MESSAGE.to_owned();
    options.images = None;
    options.metadata.insert(
        CODEX_TIMEOUT_AUTO_CONTINUE_METADATA_KEY.to_owned(),
        Value::Bool(true),
    );
    options
}

impl CodexClientSlot {
    fn new(client: CodexClient, env: HashMap<String, String>) -> Self {
        Self {
            client: Mutex::new(client),
            env,
            active_runs: AtomicUsize::new(0),
            last_used: Mutex::new(Instant::now()),
        }
    }

    fn active_run_count(&self) -> usize {
        self.active_runs.load(Ordering::SeqCst)
    }

    fn begin_run(&self) {
        self.active_runs.fetch_add(1, Ordering::SeqCst);
    }

    async fn finish_run(&self) {
        let _ = self
            .active_runs
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                Some(count.saturating_sub(1))
            });
        self.mark_used().await;
    }

    async fn mark_used(&self) {
        *self.last_used.lock().await = Instant::now();
    }

    async fn shutdown(&self) {
        self.client.lock().await.shutdown().await;
    }
}

fn schedule_idle_client_cleanup(
    clients: CodexClientMap,
    garyx_thread_id: String,
    slot: Arc<CodexClientSlot>,
    ttl: Duration,
) {
    tokio::spawn(async move {
        tokio::time::sleep(ttl).await;

        if slot.active_run_count() > 0 {
            return;
        }

        let last_used = *slot.last_used.lock().await;
        if last_used.elapsed() < ttl {
            return;
        }

        let removed = {
            let mut clients = clients.lock().await;
            if clients.get(&garyx_thread_id).is_some_and(|current| {
                Arc::ptr_eq(current, &slot) && current.active_run_count() == 0
            }) {
                clients.remove(&garyx_thread_id)
            } else {
                None
            }
        };

        if let Some(slot) = removed {
            tracing::info!(
                garyx_thread_id = %garyx_thread_id,
                idle_ttl_secs = ttl.as_secs(),
                "shutting down idle codex app-server"
            );
            slot.shutdown().await;
        }
    });
}

impl CodexAgentProvider {
    /// Create a new Codex provider with the given config.
    pub fn new(config: CodexAppServerConfig) -> Self {
        let model_defaults = std::sync::RwLock::new(ProviderModelDefaults {
            model: config.model.clone(),
            default_model: config.default_model.clone(),
            model_reasoning_effort: config.model_reasoning_effort.clone(),
            model_service_tier: config.model_service_tier.clone(),
        });
        Self {
            config,
            model_defaults,
            clients: Arc::new(Mutex::new(HashMap::new())),
            session_map: Mutex::new(HashMap::new()),
            active_runs: Mutex::new(HashMap::new()),
            active_session_turns: Mutex::new(HashMap::new()),
            active_session_callbacks: Mutex::new(HashMap::new()),
            active_session_pending_acks: Mutex::new(HashMap::new()),
            pending_rate_limits: PendingRateLimits::default(),
            thread_usage_totals: Mutex::new(HashMap::new()),
            ready: Mutex::new(false),
        }
    }

    /// Clone the frozen config with the hot-reloadable model defaults
    /// overlaid, so client construction, thread/turn request building, and
    /// runtime selection observe the latest reloaded defaults.
    fn effective_config(&self) -> CodexAppServerConfig {
        let defaults = self
            .model_defaults
            .read()
            .expect("codex model defaults lock poisoned")
            .clone();
        let mut config = self.config.clone();
        config.model = if defaults.model.is_empty() {
            defaults.default_model.clone()
        } else {
            defaults.model.clone()
        };
        config.default_model = defaults.default_model;
        config.model_reasoning_effort = defaults.model_reasoning_effort;
        config.model_service_tier = defaults.model_service_tier;
        config
    }

    fn build_client_config(&self, env: HashMap<String, String>) -> CodexClientConfig {
        let codex_bin = if self.config.codex_bin.is_empty() {
            "codex".to_owned()
        } else {
            self.config.codex_bin.clone()
        };

        let effective_config = self.effective_config();
        let model = if !effective_config.model.is_empty() {
            Some(effective_config.model.clone())
        } else if !effective_config.default_model.is_empty() {
            Some(effective_config.default_model.clone())
        } else {
            None
        };

        CodexClientConfig {
            codex_bin,
            workspace_dir: self.config.workspace_dir.clone(),
            model,
            approval_policy: self.config.approval_policy.clone(),
            sandbox_mode: self.config.sandbox_mode.clone(),
            experimental_api: self.config.experimental_api,
            request_timeout: Duration::from_secs_f64(self.config.request_timeout_seconds),
            startup_timeout: Duration::from_secs_f64(self.config.startup_timeout_seconds),
            env,
            ..CodexClientConfig::default()
        }
    }

    async fn create_client_slot(
        &self,
        env: HashMap<String, String>,
    ) -> Result<Arc<CodexClientSlot>, BridgeError> {
        let mut client = CodexClient::new(self.build_client_config(env.clone()));
        client
            .initialize()
            .await
            .map_err(|e| BridgeError::Internal(format!("codex client init failed: {e}")))?;

        Ok(Arc::new(CodexClientSlot::new(client, env)))
    }

    async fn client_for_options(
        &self,
        options: &ProviderRunOptions,
    ) -> Result<Arc<CodexClientSlot>, BridgeError> {
        let desired_env = resolve_runtime_codex_env(&self.config, &options.metadata);
        let garyx_thread_id = options.thread_id.clone();

        loop {
            let existing = self.clients.lock().await.get(&garyx_thread_id).cloned();
            if let Some(slot) = existing {
                match decide_codex_client_reuse(&slot.env, &desired_env, slot.active_run_count()) {
                    CodexClientReuseDecision::Reuse => {
                        slot.mark_used().await;
                        return Ok(slot);
                    }
                    CodexClientReuseDecision::ReplaceIdle => {
                        let removed = {
                            let mut clients = self.clients.lock().await;
                            if clients.get(&garyx_thread_id).is_some_and(|current| {
                                Arc::ptr_eq(current, &slot) && current.active_run_count() == 0
                            }) {
                                clients.remove(&garyx_thread_id)
                            } else {
                                None
                            }
                        };
                        if let Some(old_slot) = removed {
                            tracing::info!(
                                garyx_thread_id = %garyx_thread_id,
                                "restarting idle codex app-server because startup env changed"
                            );
                            old_slot.shutdown().await;
                        }
                        continue;
                    }
                }
            }

            let new_slot = self.create_client_slot(desired_env.clone()).await?;
            let mut clients = self.clients.lock().await;
            if clients.contains_key(&garyx_thread_id) {
                drop(clients);
                new_slot.shutdown().await;
                continue;
            }
            clients.insert(garyx_thread_id.clone(), new_slot.clone());
            return Ok(new_slot);
        }
    }

    async fn client_for_thread(&self, garyx_thread_id: &str) -> Option<Arc<CodexClientSlot>> {
        self.clients.lock().await.get(garyx_thread_id).cloned()
    }

    async fn finish_client_run(&self, garyx_thread_id: &str, slot: Arc<CodexClientSlot>) {
        slot.finish_run().await;
        schedule_idle_client_cleanup(
            self.clients.clone(),
            garyx_thread_id.to_owned(),
            slot,
            CODEX_CLIENT_IDLE_TTL,
        );
    }

    async fn shutdown_thread_client(&self, garyx_thread_id: &str) {
        let slot = self.clients.lock().await.remove(garyx_thread_id);
        if let Some(slot) = slot {
            slot.shutdown().await;
        }
    }

    /// Record quota-exhaustion context for a thread so the bridge run-completion
    /// path can mark the run rate-limited and schedule an automatic resend.
    /// No-op unless Codex's signal actually indicates a usage-limit failure.
    async fn stash_rate_limit_if_quota_exhausted(
        &self,
        thread_id: &str,
        usage_limit_hit: bool,
        snapshot: Option<&Value>,
        message: Option<&str>,
    ) {
        if let Some(rate_limit) = build_codex_rate_limit(
            self.config.provider_type.as_slug(),
            usage_limit_hit,
            snapshot,
            message,
        ) {
            tracing::warn!(
                thread_id = %thread_id,
                provider = %rate_limit.provider,
                window = ?rate_limit.window,
                reset_at = ?rate_limit.reset_at,
                "codex run hit usage quota; staging rate-limit context for auto-resend",
            );
            self.pending_rate_limits
                .stage(thread_id.to_owned(), rate_limit)
                .await;
        }
    }

    async fn reset_timed_out_run(&self, run_id: &str) {
        let active = self.active_runs.lock().await.get(run_id).cloned();
        let Some(active) = active else {
            return;
        };

        if let Some(client_slot) = self.client_for_thread(&active.garyx_thread_id).await {
            let client_guard = client_slot.client.lock().await;
            let interrupt_result = tokio::time::timeout(
                Duration::from_secs(10),
                client_guard.interrupt_turn(&active.codex_thread_id, &active.turn_id),
            )
            .await;
            drop(client_guard);

            match interrupt_result {
                Ok(Ok(())) => {
                    tracing::warn!(
                        run_id,
                        garyx_thread_id = %active.garyx_thread_id,
                        codex_thread_id = %active.codex_thread_id,
                        turn_id = %active.turn_id,
                        "interrupted timed-out codex turn; restarting app-server client"
                    );
                }
                Ok(Err(error)) => {
                    tracing::warn!(
                        run_id,
                        garyx_thread_id = %active.garyx_thread_id,
                        codex_thread_id = %active.codex_thread_id,
                        turn_id = %active.turn_id,
                        error = %error,
                        "failed to interrupt timed-out codex turn; restarting app-server client"
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        run_id,
                        garyx_thread_id = %active.garyx_thread_id,
                        codex_thread_id = %active.codex_thread_id,
                        turn_id = %active.turn_id,
                        "interrupting timed-out codex turn timed out; restarting app-server client"
                    );
                }
            }
        }

        self.shutdown_thread_client(&active.garyx_thread_id).await;
    }

    async fn run_streaming_once(
        &self,
        options: &ProviderRunOptions,
        live_callback: Arc<dyn Fn(StreamEvent) + Send + Sync>,
    ) -> Result<ProviderRunResult, BridgeError> {
        let client_slot = self.client_for_options(options).await?;
        client_slot.begin_run();
        let result = self
            .run_streaming_impl(options, live_callback, client_slot.clone())
            .await;
        self.finish_client_run(&options.thread_id, client_slot)
            .await;
        result
    }

    async fn cleanup_active_run_state(&self, run_id: &str) {
        self.active_runs.lock().await.remove(run_id);

        let thread_ids: Vec<String> = {
            let turns = self.active_session_turns.lock().await;
            turns
                .iter()
                .filter(|(_, (_, _, active_run_id))| active_run_id == run_id)
                .map(|(thread_id, _)| thread_id.clone())
                .collect()
        };

        let mut pending_acks = self.active_session_pending_acks.lock().await;
        pending_acks.retain(|_, (active_run_id, _)| active_run_id != run_id);
        drop(pending_acks);

        if thread_ids.is_empty() {
            return;
        }

        {
            let mut turns = self.active_session_turns.lock().await;
            for thread_id in &thread_ids {
                turns.remove(thread_id);
            }
        }

        let mut callbacks = self.active_session_callbacks.lock().await;
        for thread_id in thread_ids {
            let should_remove = callbacks
                .get(&thread_id)
                .map(|(active_run_id, _)| active_run_id == run_id)
                .unwrap_or(false);
            if should_remove {
                callbacks.remove(&thread_id);
            }
        }
    }

    async fn enqueue_streaming_input_ack(
        &self,
        garyx_thread_id: &str,
        run_id: &str,
        pending_input_id: Option<String>,
    ) -> bool {
        let Some(pending_input_id) = pending_input_id
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
        else {
            return false;
        };

        let mut pending_acks = self.active_session_pending_acks.lock().await;
        let entry = pending_acks
            .entry(garyx_thread_id.to_owned())
            .or_insert_with(|| (run_id.to_owned(), PendingAckQueue::default()));
        if entry.0 != run_id {
            *entry = (run_id.to_owned(), PendingAckQueue::default());
        }
        entry.1.enqueue(pending_input_id);
        true
    }

    async fn rollback_streaming_input_ack(
        &self,
        garyx_thread_id: &str,
        run_id: &str,
        pending_input_id: Option<&str>,
    ) {
        let Some(pending_input_id) = pending_input_id.map(str::trim).filter(|id| !id.is_empty())
        else {
            return;
        };

        let mut pending_acks = self.active_session_pending_acks.lock().await;
        if let Some((active_run_id, queue)) = pending_acks.get_mut(garyx_thread_id)
            && active_run_id == run_id
        {
            queue.rollback(pending_input_id);
        }
    }

    async fn emit_streaming_input_ack_boundary(
        &self,
        garyx_thread_id: &str,
        run_id: &str,
        pending_input_id: Option<String>,
    ) -> bool {
        let callback = {
            self.active_session_callbacks
                .lock()
                .await
                .get(garyx_thread_id)
                .and_then(|(active_run_id, callback)| {
                    if active_run_id == run_id {
                        Some(callback.clone())
                    } else {
                        None
                    }
                })
        };
        if let Some(callback) = callback {
            callback(StreamEvent::Boundary {
                kind: StreamBoundaryKind::UserAck,
                pending_input_id,
            });
            true
        } else {
            false
        }
    }

    async fn acknowledge_next_codex_user_message(
        &self,
        garyx_thread_id: &str,
        run_id: &str,
    ) -> bool {
        let marker = {
            let mut pending_acks = self.active_session_pending_acks.lock().await;
            let next = pending_acks
                .get_mut(garyx_thread_id)
                .and_then(|(active_run_id, queue)| {
                    if active_run_id == run_id {
                        queue.acknowledge_next(false)
                    } else {
                        None
                    }
                });
            if pending_acks
                .get(garyx_thread_id)
                .is_some_and(|(active_run_id, queue)| active_run_id == run_id && queue.is_empty())
            {
                pending_acks.remove(garyx_thread_id);
            }
            next
        };

        match marker {
            Some(pending_input_id) => {
                self.emit_streaming_input_ack_boundary(
                    garyx_thread_id,
                    run_id,
                    Some(pending_input_id),
                )
                .await
            }
            None => false,
        }
    }

    /// Core streaming run implementation.
    async fn run_streaming_impl(
        &self,
        options: &ProviderRunOptions,
        live_callback: Arc<dyn Fn(StreamEvent) + Send + Sync>,
        client_slot: Arc<CodexClientSlot>,
    ) -> Result<ProviderRunResult, BridgeError> {
        let client_guard = client_slot.client.lock().await;
        let client = &*client_guard;

        let run_id = resolve_run_id_with(&options.metadata, || {
            format!(
                "run_{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            )
        });

        // Drop any quota stash left by a prior run on this thread so a stale
        // entry (e.g. a usage-limit error that was followed by a successful
        // turn, which never consumes the stash) can never be attributed to this
        // run's terminal record.
        self.pending_rate_limits.clear(&options.thread_id).await;

        let start = Instant::now();
        let effective_config = self.effective_config();
        let mut actual_model = resolve_codex_actual_model(&effective_config, &options.metadata);
        let mut response_parts: Vec<String> = Vec::new();
        let mut session_messages: Vec<ProviderMessage> = Vec::new();
        let mut notification_rx = client.subscribe_events();

        // Resolve or create thread
        let sdk_session_id = options
            .metadata
            .get(SDK_SESSION_ID_METADATA_KEY)
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned());
        let fork_session = metadata_bool(&options.metadata, SDK_SESSION_FORK_METADATA_KEY);

        let existing_thread_id = {
            let session_map = self.session_map.lock().await;
            resolve_existing_thread_id(&session_map, &options.thread_id, sdk_session_id.as_deref())
        };
        // Attached to an existing Codex thread (resume or fork): its
        // cumulative token totals include prior turns.
        let thread_was_resumed = existing_thread_id.is_some();
        let include_memory = existing_thread_id.is_none() && !fork_session;
        let thread_params = build_thread_start_params(
            &effective_config,
            options.workspace_dir.as_deref(),
            &options.thread_id,
            &run_id,
            &options.metadata,
        );
        let thread_id = resume_or_start_thread(
            existing_thread_id,
            fork_session,
            thread_params,
            |params| client.resume_thread(params),
            |params| client.fork_thread(params),
            |params| client.start_thread(params),
        )
        .await?;
        self.session_map
            .lock()
            .await
            .insert(options.thread_id.clone(), thread_id.clone());
        live_callback(StreamEvent::SessionBound {
            sdk_session_id: thread_id.clone(),
        });

        // Start turn
        let turn_options = build_turn_start_options(&effective_config, &options.metadata);
        let input_items = build_input_items(options, include_memory);
        let turn_id = client
            .start_turn_with_options(&thread_id, input_items, turn_options)
            .await
            .map_err(|e| map_codex_error("turn/start failed", e))?;

        // Track active run
        {
            self.active_runs.lock().await.insert(
                run_id.clone(),
                ActiveCodexRun {
                    garyx_thread_id: options.thread_id.clone(),
                    codex_thread_id: thread_id.clone(),
                    turn_id: turn_id.clone(),
                },
            );
            self.active_session_turns.lock().await.insert(
                options.thread_id.clone(),
                (thread_id.clone(), turn_id.clone(), run_id.clone()),
            );
            self.active_session_callbacks.lock().await.insert(
                options.thread_id.clone(),
                (run_id.clone(), live_callback.clone()),
            );
            self.active_session_pending_acks.lock().await.insert(
                options.thread_id.clone(),
                (run_id.clone(), PendingAckQueue::with_root_user_message()),
            );
        }

        // Drop the client lock before entering notification loop
        drop(client_guard);

        // Notification loop
        let mut completed_turn: Option<Value> = None;
        let mut streamed_error_message: Option<String> = None;
        // Latest `account/rateLimits/updated` snapshot seen during this run, and
        // whether Codex reported a `usageLimitExceeded` error. Together these
        // let us classify a quota-exhaustion failure and read the authoritative
        // reset time straight from Codex's own structured signal.
        let mut latest_rate_limit_snapshot: Option<Value> = None;
        let mut usage_limit_hit = false;
        let mut current_agent_message_item_id: Option<String> = None;
        let mut current_agent_message_has_text = false;
        let mut thread_title: Option<String> = None;
        // As of app-server 0.144 `turn/completed` no longer carries usage, so
        // per-turn usage derives from `thread/tokenUsage/updated` snapshots.
        let mut turn_usage = CodexTurnUsageTracker::default();
        // Structured-activity items whose `item/started` produced a ToolUse
        // frame; completions without one synthesize the pair.
        let mut started_tool_item_ids: HashSet<String> = HashSet::new();

        let timeout = Duration::from_secs_f64(self.config.request_timeout_seconds);

        let loop_result: Result<(), BridgeError> = async {
            loop {
                let notification: JsonRpcNotification =
                    tokio::time::timeout(timeout, notification_rx.recv())
                        .await
                        .map_err(|_| BridgeError::Timeout)?
                        .map_err(|e| {
                            BridgeError::RunFailed(format!("notification channel error: {e}"))
                        })?;

                let method = &notification.method;
                let params = &notification.params;

                // Fatal transport error
                if method == "transport/fatal" {
                    let error_msg = params
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("codex transport fatal error")
                        .to_owned();
                    return Err(BridgeError::RunFailed(error_msg));
                }

                // Rolling rate-limit snapshots are account-scoped (no turn id),
                // so capture them before the turn-affinity gate would drop them.
                if method == "account/rateLimits/updated" {
                    if let Some(snapshot) = extract_rate_limit_snapshot(params) {
                        latest_rate_limit_snapshot = Some(snapshot);
                    }
                    continue;
                }

                // Advisory notices are account/config-scoped (no or optional
                // turn affinity), so log them before the turn-affinity gate
                // would silently drop them.
                if let Some(advisory) = codex_advisory_notification_message(method, params) {
                    let codex_thread_id =
                        params.get("threadId").and_then(Value::as_str).unwrap_or("");
                    tracing::warn!(
                        method = %method,
                        thread_id = %options.thread_id,
                        codex_thread_id = %codex_thread_id,
                        "codex app-server advisory: {advisory}"
                    );
                    continue;
                }

                // Token-usage snapshots are consumed ahead of the turn gate:
                // a snapshot replayed for a *prior* turn (resume/fork) is this
                // turn's usage baseline and would otherwise be dropped.
                if method == "thread/tokenUsage/updated" {
                    turn_usage.observe(params, &thread_id, &turn_id);
                    continue;
                }

                if !matches_turn(params, &thread_id, &turn_id) {
                    continue;
                }

                match method.as_str() {
                    "thread/name/updated" => {
                        if let Some(title) = extract_codex_thread_title(params) {
                            thread_title = Some(title);
                        }
                    }
                    "thread/started" => {
                        if thread_title.is_none()
                            && let Some(title) = extract_codex_thread_started_title(params)
                        {
                            thread_title = Some(title);
                        }
                    }
                    "item/agentMessage/delta" => {
                        maybe_emit_agent_message_separator(
                            params.get("itemId").and_then(|v| v.as_str()),
                            &mut current_agent_message_item_id,
                            &mut current_agent_message_has_text,
                            &mut response_parts,
                            live_callback.as_ref(),
                        );

                        let delta = params.get("delta").and_then(|v| v.as_str()).unwrap_or("");
                        if !delta.is_empty() {
                            response_parts.push(delta.to_owned());
                            append_codex_assistant_session_message(
                                &mut session_messages,
                                params.get("itemId").and_then(|v| v.as_str()),
                                delta,
                            );
                            live_callback(StreamEvent::Delta {
                                text: delta.to_owned(),
                            });
                            current_agent_message_has_text = true;
                        }
                    }
                    "item/started" => {
                        if let Some(item) = params.get("item") {
                            if is_user_message_item(item) {
                                // `turn/steer` only means the input reached the active turn.
                                // Codex confirms consumption by replaying it as a userMessage item.
                                self.acknowledge_next_codex_user_message(
                                    &options.thread_id,
                                    &run_id,
                                )
                                .await;
                            }
                            if is_agent_message_item(item) {
                                maybe_emit_agent_message_separator(
                                    item.get("id").and_then(|v| v.as_str()),
                                    &mut current_agent_message_item_id,
                                    &mut current_agent_message_has_text,
                                    &mut response_parts,
                                    live_callback.as_ref(),
                                );
                            }
                            if let Some(msg) = tool_session_message_for_started_item(
                                item,
                                &mut started_tool_item_ids,
                            ) {
                                emit_tool_stream_event(&msg, live_callback.as_ref());
                                session_messages.push(msg);
                            }
                        }
                    }
                    "item/completed" => {
                        if let Some(item) = params.get("item") {
                            for msg in tool_session_messages_for_completed_item(
                                item,
                                &mut started_tool_item_ids,
                            ) {
                                emit_tool_stream_event(&msg, live_callback.as_ref());
                                session_messages.push(msg);
                            }
                        }
                    }
                    "error" => {
                        if let Some(err_obj) = params.get("error") {
                            let message = err_obj
                                .get("message")
                                .and_then(|v| v.as_str())
                                .unwrap_or("codex turn error")
                                .to_owned();
                            let will_retry = params
                                .get("willRetry")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            tracing::warn!(
                                thread_id = %options.thread_id,
                                will_retry,
                                "codex turn error notification: {message}"
                            );
                            streamed_error_message = Some(message);
                            if codex_error_is_usage_limit(err_obj.get("codexErrorInfo")) {
                                usage_limit_hit = true;
                            }
                        }
                    }
                    "model/rerouted" => {
                        if let Some(to_model) = extract_rerouted_model(params) {
                            let from_model = params
                                .get("fromModel")
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            tracing::info!(
                                thread_id = %options.thread_id,
                                from_model = %from_model,
                                to_model = %to_model,
                                reason = ?params.get("reason"),
                                "codex rerouted the turn to a different model"
                            );
                            actual_model = Some(to_model);
                        }
                    }
                    "turn/completed" => {
                        let turn = params
                            .get("turn")
                            .cloned()
                            .unwrap_or(Value::Object(serde_json::Map::new()));
                        completed_turn = Some(turn);
                        break;
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        .await;

        if matches!(&loop_result, Err(BridgeError::Timeout)) {
            self.reset_timed_out_run(&run_id).await;
        }

        // Cleanup tracking
        self.cleanup_active_run_state(&run_id).await;

        // Read the usage baseline remembered from the previous in-process turn
        // and remember this turn's cumulative totals for the next one, even
        // when the run fails: the tokens were consumed either way.
        let stored_usage_baseline = {
            let mut totals = self.thread_usage_totals.lock().await;
            let stored = totals.get(&thread_id).copied();
            if let Some(latest) = turn_usage.latest_totals() {
                totals.insert(thread_id.clone(), max_totals(stored, latest));
            }
            stored
        };

        let duration_ms = start.elapsed().as_millis() as i64;
        let response = response_parts.join("");

        // If the loop errored, return a failure result
        if let Err(e) = loop_result {
            tracing::error!(error = %e, "codex provider run_streaming error");
            self.stash_rate_limit_if_quota_exhausted(
                &options.thread_id,
                usage_limit_hit,
                latest_rate_limit_snapshot.as_ref(),
                streamed_error_message.as_deref(),
            )
            .await;
            return Ok(ProviderRunResult {
                run_id,
                thread_id: options.thread_id.clone(),
                response,
                session_messages,
                sdk_session_id: Some(thread_id),
                actual_model,
                thread_title: None,
                success: false,
                error: Some(e.to_string()),
                input_tokens: 0,
                output_tokens: 0,
                cost: 0.0,
                duration_ms,
            });
        }

        // Build result from completed turn
        let completed = completed_turn.unwrap_or(Value::Object(serde_json::Map::new()));
        let status = completed
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("completed")
            .to_lowercase();
        let success = status != "failed";

        let error = if status == "failed" {
            let from_turn = completed
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned());
            Some(
                from_turn
                    .or(streamed_error_message)
                    .unwrap_or_else(|| "codex turn failed".to_owned()),
            )
        } else {
            None
        };

        if !success {
            tracing::warn!(
                run_id = %run_id,
                thread_id = %options.thread_id,
                sdk_session_id = %thread_id,
                status = %status,
                error = %error.as_deref().unwrap_or("unknown codex turn failure"),
                "codex turn completed with failure",
            );
            self.stash_rate_limit_if_quota_exhausted(
                &options.thread_id,
                usage_limit_hit,
                latest_rate_limit_snapshot.as_ref(),
                error.as_deref(),
            )
            .await;
        }

        let (input_tokens, output_tokens, cost) = {
            let (input_tokens, output_tokens, cost) = extract_usage(&completed);
            if input_tokens == 0 && output_tokens == 0 {
                // `turn/completed` omits usage on app-server >= 0.144; derive
                // this turn's usage from its token-usage snapshots instead.
                let (snapshot_input, snapshot_output) =
                    turn_usage.finish(stored_usage_baseline, thread_was_resumed);
                (snapshot_input, snapshot_output, cost)
            } else {
                (input_tokens, output_tokens, cost)
            }
        };

        live_callback(StreamEvent::Done);

        Ok(ProviderRunResult {
            run_id,
            thread_id: options.thread_id.clone(),
            response,
            session_messages,
            sdk_session_id: Some(thread_id),
            actual_model,
            thread_title,
            success,
            error,
            input_tokens,
            output_tokens,
            cost,
            duration_ms,
        })
    }
}

#[async_trait]
impl ProviderRuntime for CodexAgentProvider {
    fn provider_type(&self) -> ProviderType {
        // Codex and Traex share this provider implementation; report the
        // configured identity so thread affinity, presentation, and session
        // persistence treat them as distinct providers.
        self.config.provider_type.clone()
    }

    fn is_ready(&self) -> bool {
        // Use try_lock to avoid blocking; if lock is held, provider is busy but ready
        self.ready.try_lock().map(|g| *g).unwrap_or(false)
    }

    fn resolve_runtime_selection(&self, options: &ProviderRunOptions) -> ProviderRuntimeSelection {
        let effective_config = self.effective_config();
        ProviderRuntimeSelection {
            model: resolve_codex_request_model(&effective_config, &options.metadata),
            model_reasoning_effort: resolve_codex_request_reasoning_effort(
                &effective_config,
                &options.metadata,
            ),
            model_service_tier: resolve_codex_request_service_tier(
                &effective_config,
                &options.metadata,
            ),
        }
    }

    fn update_model_defaults(&self, defaults: &ProviderModelDefaults) {
        *self
            .model_defaults
            .write()
            .expect("codex model defaults lock poisoned") = defaults.clone();
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        if *self.ready.lock().await {
            return Ok(());
        }

        *self.ready.lock().await = true;
        tracing::info!("codex provider initialized; app-server clients are started per thread");
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        tracing::info!("shutting down codex provider");

        let clients: Vec<Arc<CodexClientSlot>> = {
            let mut clients = self.clients.lock().await;
            clients.drain().map(|(_, slot)| slot).collect()
        };
        for client in clients {
            client.shutdown().await;
        }

        self.active_runs.lock().await.clear();
        self.active_session_turns.lock().await.clear();
        self.active_session_callbacks.lock().await.clear();
        self.active_session_pending_acks.lock().await.clear();

        *self.ready.lock().await = false;
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        if !*self.ready.lock().await {
            return Err(BridgeError::ProviderNotReady);
        }
        let live_callback: Arc<dyn Fn(StreamEvent) + Send + Sync> = on_chunk.into();
        let result = self
            .run_streaming_once(options, live_callback.clone())
            .await;

        if codex_run_result_timed_out(&result)
            && !options
                .metadata
                .get(CODEX_TIMEOUT_AUTO_CONTINUE_METADATA_KEY)
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            tracing::warn!(
                thread_id = %options.thread_id,
                "codex provider timed out; restarting app-server and retrying once with continue"
            );
            let continue_options = codex_timeout_auto_continue_options(options);
            return self
                .run_streaming_once(&continue_options, live_callback)
                .await;
        }

        result
    }

    async fn take_rate_limit(&self, thread_id: &str) -> Option<ProviderRateLimit> {
        self.pending_rate_limits.take(thread_id).await
    }

    async fn abort(&self, run_id: &str) -> bool {
        let active = self.active_runs.lock().await.get(run_id).cloned();
        let Some(active) = active else {
            self.cleanup_active_run_state(run_id).await;
            return false;
        };

        let Some(client_slot) = self.client_for_thread(&active.garyx_thread_id).await else {
            self.cleanup_active_run_state(run_id).await;
            return false;
        };

        let client_guard = client_slot.client.lock().await;
        // Try interrupt with timeout; force-cleanup on failure
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            client_guard.interrupt_turn(&active.codex_thread_id, &active.turn_id),
        )
        .await;

        match result {
            Ok(Ok(())) => {
                self.cleanup_active_run_state(run_id).await;
                true
            }
            Ok(Err(e)) => {
                tracing::warn!(run_id, error = %e, "codex abort failed");
                self.cleanup_active_run_state(run_id).await;
                false
            }
            Err(_) => {
                tracing::warn!(run_id, "codex abort timed out, force-cleaning up");
                self.cleanup_active_run_state(run_id).await;
                false
            }
        }
    }

    fn supports_streaming_input(&self) -> bool {
        true
    }

    async fn add_streaming_input(&self, thread_id: &str, input: QueuedUserInput) -> bool {
        let garyx_thread_id = thread_id.to_owned();
        let active = {
            self.active_session_turns
                .lock()
                .await
                .get(&garyx_thread_id)
                .cloned()
        };

        let Some((codex_thread_id, turn_id, run_id)) = active else {
            return false;
        };

        let Some(client_slot) = self.client_for_thread(&garyx_thread_id).await else {
            return false;
        };

        let pending_input_id = input.pending_input_id.clone();
        self.enqueue_streaming_input_ack(&garyx_thread_id, &run_id, pending_input_id.clone())
            .await;
        let input = build_input_items_from_parts(&input.message, &input.images, &input.attachments);

        let steer_timeout = configured_request_timeout(self.config.request_timeout_seconds)
            .min(CODEX_STREAMING_INPUT_STEER_TIMEOUT);
        let client_guard = client_slot.client.lock().await;
        match client_guard
            .steer_turn_with_timeout(&codex_thread_id, &turn_id, input, steer_timeout)
            .await
        {
            Ok(()) => {
                tracing::debug!(
                    garyx_thread_id = %garyx_thread_id,
                    codex_thread_id = %codex_thread_id,
                    run_id = %run_id,
                    timeout_ms = steer_timeout.as_millis() as u64,
                    "steered codex turn with additional input; waiting for userMessage item ack"
                );
                true
            }
            Err(e) => {
                self.rollback_streaming_input_ack(
                    &garyx_thread_id,
                    &run_id,
                    pending_input_id.as_deref(),
                )
                .await;
                tracing::warn!(
                    garyx_thread_id = %garyx_thread_id,
                    codex_thread_id = %codex_thread_id,
                    run_id = %run_id,
                    timeout_ms = steer_timeout.as_millis() as u64,
                    error = %e,
                    "failed to steer codex turn"
                );
                false
            }
        }
    }

    async fn interrupt_streaming_session(&self, thread_id: &str) -> bool {
        let active = {
            self.active_session_turns
                .lock()
                .await
                .get(thread_id)
                .cloned()
        };

        let Some((_thread_id, _turn_id, run_id)) = active else {
            return false;
        };

        self.abort(&run_id).await
    }

    async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
        let map = self.session_map.lock().await;
        if let Some(existing_thread_id) = map.get(thread_id) {
            return Ok(existing_thread_id.clone());
        }
        // No existing thread - return a placeholder; actual thread creation
        // happens in run() via thread/start.
        Ok(String::new())
    }

    async fn clear_session(&self, thread_id: &str) -> ClearSessionOutcome {
        let mut removed = self.session_map.lock().await.remove(thread_id).is_some();
        removed |= self
            .active_session_turns
            .lock()
            .await
            .remove(thread_id)
            .is_some();
        removed |= self
            .active_session_callbacks
            .lock()
            .await
            .remove(thread_id)
            .is_some();
        removed |= self
            .active_session_pending_acks
            .lock()
            .await
            .remove(thread_id)
            .is_some();
        self.shutdown_thread_client(thread_id).await;
        if removed {
            ClearSessionOutcome::Cleared
        } else {
            ClearSessionOutcome::AlreadyAbsent
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
