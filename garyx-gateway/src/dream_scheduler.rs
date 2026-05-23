use std::sync::Arc;

use chrono::{Duration, Utc};
use tokio::time::{MissedTickBehavior, interval};

use crate::dreams::{DreamAutoScanOutcome, run_auto_dream_scan_once};
use crate::server::AppState;

const DREAM_SCHEDULER_POLL_SECS: u64 = 60;
const MIN_AUTO_SCAN_INTERVAL_SECS: u64 = 60;
const MAX_AUTO_SCAN_INTERVAL_SECS: u64 = 24 * 60 * 60;

pub(crate) fn spawn_scheduler(state: Arc<AppState>) {
    tokio::spawn(async move {
        run_scheduler(state).await;
    });
}

async fn run_scheduler(state: Arc<AppState>) {
    let mut ticker = interval(std::time::Duration::from_secs(DREAM_SCHEDULER_POLL_SECS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut next_due = Utc::now();

    loop {
        ticker.tick().await;

        let config = state.config_snapshot();
        let interval_secs = config
            .dreams
            .scan_interval_secs
            .clamp(MIN_AUTO_SCAN_INTERVAL_SECS, MAX_AUTO_SCAN_INTERVAL_SECS);
        if !config.dreams.enabled {
            next_due = Utc::now();
            continue;
        }

        let now = Utc::now();
        if now < next_due {
            continue;
        }
        next_due = now + Duration::seconds(interval_secs as i64);

        match run_auto_dream_scan_once(&state, now).await {
            Ok(DreamAutoScanOutcome::Disabled) => {}
            Ok(DreamAutoScanOutcome::NoRecentMessages { from, to }) => {
                tracing::debug!(from, to, "dream auto scan skipped: no recent user messages");
            }
            Ok(DreamAutoScanOutcome::Scanned {
                run_id,
                topics_count,
                spans_count,
                matched_messages,
            }) => {
                tracing::info!(
                    run_id,
                    topics_count,
                    spans_count,
                    matched_messages,
                    "dream auto scan completed"
                );
            }
            Err(error) => {
                tracing::warn!(error = %error, "dream auto scan failed");
            }
        }
    }
}
