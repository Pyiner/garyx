//! Single derivation point for the hot-reloadable subset of `GaryxConfig`.
//!
//! Initial assembly and `AppState::apply_runtime_config` used to derive these
//! values independently, which is how the meetings ingestion window drifted
//! out of the hot-reload path. Both consumers now compute one
//! [`RuntimeConfigProjection`] per config snapshot and read the derived
//! values from it, so the two paths cannot disagree.
//!
//! Deliberately excluded (do not add these here):
//! - bridge provider topology — `reload_from_config` owns its own
//!   reconciliation against the live pool;
//! - cron job definitions — boot-only, `CronService::load` runs once;
//! - agent profiles — sourced from the custom-agent store, not the config;
//! - `gateway.host`/`port` — boot-only reads (`public_url` is a
//!   channel-plugin rebuild input, `plugins.auto_update` is projected
//!   below as a hot knob);
//! - `config.channels` — passed through verbatim because the hot-reload
//!   dispatcher rebuild has state (weixin running, preserved plugin senders)
//!   that a pre-derived value must not flatten.
//! - the default agent id — router-owned via
//!   `garyx_router::default_agent_from_config`, the shared derivation both
//!   router construction and `update_config` call.

use garyx_models::config::GaryxConfig;

/// Values derived from `GaryxConfig` that both initial assembly and runtime
/// hot-reload consume.
#[derive(Debug, Clone)]
pub struct RuntimeConfigProjection {
    /// Meetings transcript read page size (`gateway.meetings`), clamped by
    /// `effective_read_page_bytes`.
    pub meeting_read_page_bytes: usize,
    /// Meetings ingestion join-retry window (`gateway.meetings`), clamped by
    /// `effective_join_retry_window_secs`.
    pub meeting_join_retry_window_secs: u64,
    /// Plugin self-update master switch (`plugins.auto_update`). Hot:
    /// applied to the process-wide shared AtomicBool on every config
    /// apply — never requires a channel-plugin rebuild.
    pub plugin_auto_update: bool,
}

impl RuntimeConfigProjection {
    pub fn from_config(config: &GaryxConfig) -> Self {
        Self {
            meeting_read_page_bytes: config.gateway.meetings.effective_read_page_bytes(),
            meeting_join_retry_window_secs: config
                .gateway
                .meetings
                .effective_join_retry_window_secs(),
            plugin_auto_update: config.plugins.auto_update,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sub-floor inputs must come out clamped: if the projection ever
    /// switches to the raw config fields, these assertions go red.
    #[test]
    fn meeting_values_clamp_sub_floor_inputs() {
        let mut config = GaryxConfig::default();
        config.gateway.meetings.read_page_bytes = 0;
        config.gateway.meetings.join_retry_window_secs = 0;
        let projection = RuntimeConfigProjection::from_config(&config);
        assert_eq!(projection.meeting_read_page_bytes, 4_096);
        assert_eq!(projection.meeting_join_retry_window_secs, 1);
    }

    #[test]
    fn meeting_values_pass_through_above_floor() {
        let mut config = GaryxConfig::default();
        config.gateway.meetings.read_page_bytes = 8_192;
        config.gateway.meetings.join_retry_window_secs = 120;
        let projection = RuntimeConfigProjection::from_config(&config);
        assert_eq!(projection.meeting_read_page_bytes, 8_192);
        assert_eq!(projection.meeting_join_retry_window_secs, 120);
    }
}
