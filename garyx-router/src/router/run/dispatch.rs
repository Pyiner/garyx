use super::super::*;

impl MessageRouter {
    /// Create a response callback wrapper that records outbound messages
    /// for reply routing.
    ///
    /// The wrapper intercepts `StreamEvent::Done` and calls
    /// `record_outbound_fn` with each outbound message id. This is
    /// used by channel handlers that need reply routing to work with
    /// `route_and_dispatch`.
    ///
    /// `record_outbound_fn` is called with `(thread_id, channel, account_id, message_id)`.
    pub fn wrap_response_callback<F>(
        inner: Arc<dyn Fn(StreamEvent) + Send + Sync>,
        record_outbound_fn: F,
    ) -> Arc<dyn Fn(StreamEvent) + Send + Sync>
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        Arc::new(move |event: StreamEvent| {
            // Forward to inner callback
            inner(event.clone());

            // On Done, give the channel handler a chance to record
            // outbound message ids. The actual recording happens in
            // the closure provided by the caller since only the channel
            // handler knows the platform message ids.
            if matches!(event, StreamEvent::Done) {
                record_outbound_fn("");
            }
        })
    }

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
