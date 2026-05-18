//! Background self-update loop for the gateway binary.
//!
//! Mirrors `plugins_cli::auto_update_loop` but for the gateway
//! itself instead of subprocess plugins. Spawned from `run_gateway`
//! at boot when `gateway.auto_update.enabled = true`.
//!
//! Each tick:
//!   1. fetch the GitHub Releases `latest` tag for `github_repo`
//!   2. [`should_upgrade`] gate — strict-greater-than, never
//!      downgrades (fixes the regression that bit plugin auto-update)
//!   3. [`wait_for_stream_idle`] — no in-flight dispatches across
//!      all subprocess plugins for `stream_idle_required_secs` in a
//!      row, polled every `stream_idle_poll_interval_secs`, with a
//!      `stream_idle_max_wait_secs` cap so a never-quiet gateway
//!      eventually gives up and the next tick retries
//!   4. [`try_swap_garyx_binary`] — download release, verify sha256,
//!      codesign on macOS, atomically rename into the install path
//!   5. `std::process::exit(0)` — the OS supervisor (launchd
//!      KeepAlive=true on macOS, systemd `Restart=always` on Linux)
//!      relaunches us on the new binary. Abrupt exit is acceptable
//!      because step 3 already guaranteed no in-flight work.
//!
//! Every step that fails just warn-logs and returns from the tick;
//! the next tick retries. Auto-update is best-effort and must never
//! panic or stall the gateway.

use std::sync::Arc;
use std::time::Duration;

use garyx_channels::plugin::ChannelPluginManager;
use garyx_models::config::GatewayAutoUpdateConfig;
use reqwest::Client;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::auto_update_common::{
    IdleGateConfig, IdleWaitError, should_upgrade, wait_for_stream_idle,
};
use crate::commands::{
    VERSION, github_token_from_env, latest_release_version_for_repo, replacement_binary_path,
    try_swap_garyx_binary,
};

/// Delay before the first tick. Lets boot work (config load, plugin
/// registration, RPC server bind) settle so the very first tick
/// isn't competing for fd / DB connection. Mirrors plugin
/// auto-update loop's 8s value.
const INITIAL_CHECK_DELAY: Duration = Duration::from_secs(8);

/// Lower bound on the recurring tick. Operators can shorten the
/// interval in `garyx.json` for tests but anything below this hammers
/// the GitHub API's unauthenticated rate limit (60 req/h).
const MIN_INTERVAL_SECS: u64 = 60;

/// Spawn the auto-update loop. Returns `None` when disabled in
/// config so the caller doesn't need to keep a join handle.
pub fn spawn(
    plugin_manager: Arc<Mutex<ChannelPluginManager>>,
    config: GatewayAutoUpdateConfig,
) -> Option<JoinHandle<()>> {
    if !config.enabled {
        info!("gateway auto-update disabled by config");
        return None;
    }
    Some(tokio::spawn(async move {
        run(plugin_manager, config).await;
    }))
}

async fn run(plugin_manager: Arc<Mutex<ChannelPluginManager>>, config: GatewayAutoUpdateConfig) {
    let interval = Duration::from_secs(config.check_interval_secs.max(MIN_INTERVAL_SECS));
    info!(
        installed = %VERSION,
        initial_delay_secs = INITIAL_CHECK_DELAY.as_secs(),
        interval_secs = interval.as_secs(),
        github_repo = %config.github_repo,
        "gateway auto-update loop scheduled"
    );
    tokio::time::sleep(INITIAL_CHECK_DELAY).await;
    loop {
        tick(&plugin_manager, &config).await;
        // A successful tick already called `exit(0)` and this future
        // is being torn down; the sleep below only runs when the
        // tick was a no-op, a transient failure, or got blocked by
        // the stream-idle gate's max_wait cap. In all of those cases
        // we want backpressure rather than a tight retry loop.
        tokio::time::sleep(interval).await;
    }
}

async fn tick(
    plugin_manager: &Arc<Mutex<ChannelPluginManager>>,
    config: &GatewayAutoUpdateConfig,
) {
    let client = match Client::builder()
        .user_agent(format!("garyx-cli/{VERSION}"))
        .build()
    {
        Ok(c) => c,
        Err(err) => {
            warn!(error = %err, "gateway auto-update: http client build failed");
            return;
        }
    };

    // Codex review #6: treat blank `github_repo` (operator wrote
    // `""` in garyx.json) as "use the compile-time default" rather
    // than letting it form a `https://api.github.com/repos//...` URL.
    // Trim defensively so trailing whitespace doesn't poison the URL
    // either.
    let configured_repo = config.github_repo.trim();
    let effective_repo = if configured_repo.is_empty() {
        crate::commands::GITHUB_RELEASE_REPO_DEFAULT
    } else {
        configured_repo
    };

    let token = github_token_from_env();
    let latest = match latest_release_version_for_repo(&client, effective_repo, token.as_deref()).await {
        Ok(v) => v,
        Err(err) => {
            warn!(
                error = %err,
                github_repo = %effective_repo,
                "gateway auto-update: failed to fetch latest release"
            );
            return;
        }
    };

    if !should_upgrade(VERSION, &latest) {
        info!(
            installed = %VERSION,
            latest = %latest,
            "gateway auto-update: no upgrade available"
        );
        return;
    }

    info!(
        installed = %VERSION,
        latest = %latest,
        "gateway auto-update: new release detected; entering stream-idle gate"
    );

    let idle_config = IdleGateConfig {
        required_idle_secs: config.stream_idle_required_secs,
        poll_interval_secs: config.stream_idle_poll_interval_secs,
        max_wait_secs: config.stream_idle_max_wait_secs,
    };
    match wait_for_stream_idle(plugin_manager, idle_config).await {
        Ok(()) => {}
        Err(IdleWaitError::Timeout { .. }) => {
            // Already warn-logged by wait_for_stream_idle. The next
            // tick retries from the top (re-fetch latest, re-check).
            return;
        }
    }

    let destination = match replacement_binary_path(None) {
        Ok(p) => p,
        Err(err) => {
            warn!(
                error = %err,
                "gateway auto-update: failed to resolve install path"
            );
            return;
        }
    };

    info!(
        installed = %VERSION,
        latest = %latest,
        target = %destination.display(),
        "gateway auto-update: stream-idle confirmed; starting swap"
    );

    let outcome = match try_swap_garyx_binary(&latest, effective_repo, &destination).await {
        Ok(o) => o,
        Err(err) => {
            warn!(
                installed = %VERSION,
                latest = %latest,
                error = %err,
                "gateway auto-update: swap failed; will retry next tick"
            );
            return;
        }
    };

    info!(
        from = %outcome.from_version,
        to = %outcome.to_version,
        install_path = %outcome.install_path.display(),
        "gateway auto-update: swap succeeded; exiting for supervisor to relaunch"
    );
    // The OS supervisor (launchd KeepAlive=true / systemd
    // Restart=always) is responsible for relaunching us. We use
    // `exit(0)` rather than SIGTERM-then-graceful because step 3
    // above already drained in-flight work — there's nothing left
    // to gracefully shut down. Skipping the shutdown handler keeps
    // the swap window short.
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_interval_is_one_minute() {
        // Documents the rate-limit floor. If anyone bumps this they
        // also need to bump the user-facing default in
        // `GatewayAutoUpdateConfig::default()`.
        assert_eq!(MIN_INTERVAL_SECS, 60);
    }

    #[test]
    fn initial_delay_matches_plugin_loop() {
        // Both loops boot at the same cadence so the very first
        // tick doesn't race the gateway's RPC server coming up.
        assert_eq!(INITIAL_CHECK_DELAY, Duration::from_secs(8));
    }

    #[test]
    fn spawn_returns_none_when_disabled() {
        let plugin_manager = Arc::new(Mutex::new(ChannelPluginManager::new()));
        let config = GatewayAutoUpdateConfig {
            enabled: false,
            ..GatewayAutoUpdateConfig::default()
        };
        assert!(spawn(plugin_manager, config).is_none());
    }
}
