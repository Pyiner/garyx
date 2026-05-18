//! Shared helpers for the gateway + plugin auto-update flows.
//!
//! Both flows need the same two primitives:
//!
//! 1. A strict-greater-than version comparator. The plugin flow had a
//!    historical bug where an upstream manifest advertising an older
//!    version than the installed bundle would silently downgrade the
//!    plugin; the gateway flow must not repeat that. `should_upgrade`
//!    is the single decision point — both callers route their
//!    install-vs-advertised comparison through it.
//!
//! 2. A "no in-flight stream + N consecutive idle seconds" gate. The
//!    gateway and plugin both tear down subprocess channels when they
//!    swap binaries, so neither can do that mid-dispatch without
//!    cutting off whatever the user is currently waiting on. The gate
//!    polls [`ChannelPluginManager::total_active_streams`] every
//!    `poll_interval_secs`, resetting the idle timer whenever a fresh
//!    stream appears, and only returns `Ok(())` once the count has
//!    been zero for `required_idle_secs` in a row. `max_wait_secs`
//!    bounds the wait so a never-idle gateway eventually gives up
//!    and the next tick retries.

use std::sync::Arc;
use std::time::{Duration, Instant};

use garyx_channels::plugin::ChannelPluginManager;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Decide whether `latest` is a strictly higher version than
/// `installed`. Both are parsed as dot-separated integer segments
/// (`"0.1.23"` → `[0, 1, 23]`). Leading `v` is stripped from
/// `latest` because GitHub release `tag_name` is conventionally
/// `v0.1.23` while `CARGO_PKG_VERSION` is `0.1.23`.
///
/// Returns false (= skip upgrade) when:
///   * either side fails to parse — `0.1.23-rc1`, `2026.05.18`,
///     `unknown`, empty string. Conservatively don't auto-mutate.
///   * `installed == latest` — already on the advertised version.
///   * `installed > latest` — local build ahead of the release.
///     Could be a dev build, an aborted rollback, or a CDN regression.
///     Either way: never downgrade.
pub fn should_upgrade(installed: &str, latest: &str) -> bool {
    let normalized_latest = latest.trim().trim_start_matches('v');
    let installed_parts = parse_int_segments(installed.trim());
    let latest_parts = parse_int_segments(normalized_latest);
    match (installed_parts, latest_parts) {
        (Some(i), Some(l)) if l > i => true,
        (Some(_), Some(_)) => false,
        _ => false,
    }
}

fn parse_int_segments(value: &str) -> Option<Vec<u64>> {
    if value.is_empty() {
        return None;
    }
    value
        .split('.')
        .map(|seg| seg.parse::<u64>().ok())
        .collect()
}

/// Configuration for [`wait_for_stream_idle`]. All durations are
/// expressed in seconds because that's how the on-disk config schema
/// stores them.
#[derive(Debug, Clone, Copy)]
pub struct IdleGateConfig {
    /// Number of consecutive seconds the `total_active_streams` count
    /// must remain zero before the gate releases.
    pub required_idle_secs: u64,
    /// How often to poll the plugin manager.
    pub poll_interval_secs: u64,
    /// Maximum wall-clock seconds the gate is allowed to spin. If the
    /// system never quiets down within this budget the gate gives up
    /// with [`IdleWaitError::Timeout`] and the caller is expected to
    /// retry on the next tick.
    pub max_wait_secs: u64,
}

impl Default for IdleGateConfig {
    fn default() -> Self {
        Self {
            required_idle_secs: 60,
            poll_interval_secs: 5,
            max_wait_secs: 24 * 60 * 60,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IdleWaitError {
    #[error("stream-idle gate timed out after {waited_secs}s (max_wait_secs={max_wait_secs})")]
    Timeout {
        waited_secs: u64,
        max_wait_secs: u64,
    },
}

/// Block until the plugin manager reports zero active streams across
/// every registered subprocess plugin for `config.required_idle_secs`
/// consecutive seconds, or until `config.max_wait_secs` elapses.
///
/// Any new stream appearing mid-wait resets the idle timer — the
/// auto-update caller must observe a true lull, not just a momentary
/// gap between two back-to-back dispatches.
pub async fn wait_for_stream_idle(
    plugin_manager: &Arc<Mutex<ChannelPluginManager>>,
    config: IdleGateConfig,
) -> Result<(), IdleWaitError> {
    let started = Instant::now();
    let max_wait = Duration::from_secs(config.max_wait_secs);
    let required_idle = Duration::from_secs(config.required_idle_secs);
    let poll = Duration::from_secs(config.poll_interval_secs.max(1));
    let mut idle_since: Option<Instant> = None;

    info!(
        required_idle_secs = config.required_idle_secs,
        poll_interval_secs = config.poll_interval_secs,
        max_wait_secs = config.max_wait_secs,
        "auto-update: waiting for stream-idle"
    );

    loop {
        let waited = started.elapsed();
        if waited >= max_wait {
            warn!(
                waited_secs = waited.as_secs(),
                max_wait_secs = config.max_wait_secs,
                "auto-update: stream-idle gate exhausted budget; giving up this tick"
            );
            return Err(IdleWaitError::Timeout {
                waited_secs: waited.as_secs(),
                max_wait_secs: config.max_wait_secs,
            });
        }

        let count = plugin_manager.lock().await.total_active_streams();
        if count == 0 {
            let since = *idle_since.get_or_insert_with(Instant::now);
            let idle_for = since.elapsed();
            if idle_for >= required_idle {
                info!(
                    idle_secs = idle_for.as_secs(),
                    waited_secs = waited.as_secs(),
                    "auto-update: stream-idle confirmed"
                );
                return Ok(());
            }
            debug!(
                idle_secs = idle_for.as_secs(),
                target_secs = config.required_idle_secs,
                "auto-update: stream-idle accumulating"
            );
        } else {
            if idle_since.is_some() {
                debug!(
                    active_streams = count,
                    "auto-update: idle timer reset by new stream"
                );
            } else {
                debug!(
                    active_streams = count,
                    "auto-update: waiting on active streams"
                );
            }
            idle_since = None;
        }

        tokio::time::sleep(poll).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_upgrade_strict_greater() {
        // Standard upgrade direction.
        assert!(should_upgrade("0.1.23", "0.1.24"));
        assert!(should_upgrade("0.1.23", "0.2.0"));
        assert!(should_upgrade("0.1.23", "1.0.0"));
        // GitHub tag_name prefix is stripped.
        assert!(should_upgrade("0.1.23", "v0.1.24"));
        // Whitespace tolerance.
        assert!(should_upgrade("  0.1.23  ", " v0.1.24"));
    }

    #[test]
    fn should_upgrade_refuses_equal() {
        assert!(!should_upgrade("0.1.23", "0.1.23"));
        assert!(!should_upgrade("0.1.23", "v0.1.23"));
    }

    #[test]
    fn should_upgrade_refuses_downgrade() {
        // Installed ahead of latest — never auto-replace. Common
        // when running a local dev build, or when a stale `latest.json`
        // points at an older release than the operator already
        // hand-installed.
        assert!(!should_upgrade("0.1.24", "0.1.23"));
        assert!(!should_upgrade("0.2.0", "0.1.99"));
        assert!(!should_upgrade("1.0.0", "0.99.99"));
    }

    #[test]
    fn should_upgrade_refuses_unparseable() {
        // Pre-release / build-metadata suffixes are not int-parseable
        // segments. Conservative: skip rather than guess. Manual
        // `garyx update --version` is the escape hatch.
        assert!(!should_upgrade("0.1.23-rc1", "0.1.24"));
        assert!(!should_upgrade("0.1.23", "0.1.24-rc1"));
        assert!(!should_upgrade("dev", "0.1.24"));
        assert!(!should_upgrade("0.1.23", "unknown"));
        assert!(!should_upgrade("", "0.1.24"));
        assert!(!should_upgrade("0.1.23", ""));
    }

    #[test]
    fn should_upgrade_segment_count_difference() {
        // Lexicographic compare on segment Vec handles unequal
        // segment counts. `0.1` < `0.1.1` because the longer vec
        // sorts after the shorter when prefixes are equal.
        assert!(should_upgrade("0.1", "0.1.1"));
        assert!(!should_upgrade("0.1.1", "0.1"));
        assert!(!should_upgrade("0.1.0", "0.1"));
    }
}
