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
//! - `gateway.host`/`port`/`public_url` and the auto-update switches —
//!   boot-only reads;
//! - `config.channels` — passed through verbatim because the hot-reload
//!   dispatcher rebuild has state (weixin running, preserved plugin senders)
//!   that a pre-derived value must not flatten.

use std::collections::HashMap;

use garyx_models::config::{GaryxConfig, McpServerConfig};

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
    /// The default agent id (`agents["default"]`, falling back to `"main"`).
    pub default_agent: String,
    /// Managed MCP server definitions handed to the cron dispatch runtime.
    pub managed_mcp_servers: HashMap<String, McpServerConfig>,
}

impl RuntimeConfigProjection {
    pub fn from_config(config: &GaryxConfig) -> Self {
        Self {
            meeting_read_page_bytes: config.gateway.meetings.effective_read_page_bytes(),
            meeting_join_retry_window_secs: config
                .gateway
                .meetings
                .effective_join_retry_window_secs(),
            default_agent: config
                .agents
                .get("default")
                .and_then(|v| v.as_str())
                .unwrap_or("main")
                .to_owned(),
            managed_mcp_servers: config.mcp_servers.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_agent_falls_back_to_main() {
        let config = GaryxConfig::default();
        let projection = RuntimeConfigProjection::from_config(&config);
        assert_eq!(projection.default_agent, "main");
    }

    #[test]
    fn default_agent_reads_configured_value() {
        let mut config = GaryxConfig::default();
        config
            .agents
            .insert("default".to_owned(), serde_json::json!("gary"));
        let projection = RuntimeConfigProjection::from_config(&config);
        assert_eq!(projection.default_agent, "gary");
    }

    /// Equivalence pin: the projection's default-agent derivation must match
    /// what `MessageRouter` derives for the same config, both at construction
    /// and through `update_config`. This guards the currently-duplicated
    /// derivation sites against drift.
    #[test]
    fn default_agent_matches_router_derivation() {
        let mut config = GaryxConfig::default();
        config
            .agents
            .insert("default".to_owned(), serde_json::json!("gary"));
        let projection = RuntimeConfigProjection::from_config(&config);

        let store: std::sync::Arc<dyn garyx_router::ThreadStore> =
            std::sync::Arc::new(garyx_router::InMemoryThreadStore::new());
        let mut router = garyx_router::MessageRouter::new(store, GaryxConfig::default());
        router.update_config(config);
        assert_eq!(
            router.resolve_agent_for_channel("api", "main", None, false),
            projection.default_agent
        );
    }

    #[test]
    fn meeting_values_apply_effective_clamps() {
        let config = GaryxConfig::default();
        let projection = RuntimeConfigProjection::from_config(&config);
        assert_eq!(
            projection.meeting_read_page_bytes,
            config.gateway.meetings.effective_read_page_bytes()
        );
        assert_eq!(
            projection.meeting_join_retry_window_secs,
            config.gateway.meetings.effective_join_retry_window_secs()
        );
    }
}
