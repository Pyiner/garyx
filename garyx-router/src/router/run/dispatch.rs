use super::super::*;

impl MessageRouter {
    // ------------------------------------------------------------------
    // Config
    // ------------------------------------------------------------------

    /// Update configuration (for hot reload).
    pub fn update_config(&mut self, config: GaryxConfig) {
        self.default_agent = config
            .agents
            .get("default")
            .and_then(|v| v.as_str())
            .unwrap_or("main")
            .to_owned();
        self.config = config;
    }

    /// Check if a thread id represents a scheduled cron thread rather than
    /// a user-interactive one.
    pub fn is_scheduled_thread(thread_id: &str) -> bool {
        thread_id.starts_with("cron::")
    }
}
