use super::super::*;
use garyx_models::messages::MessageMetadata;
use std::collections::HashMap;
use tracing::{debug, info};

impl MessageRouter {
    // ------------------------------------------------------------------
    // Agent / route resolution
    // ------------------------------------------------------------------

    /// Resolve which agent should handle messages for a given channel context.
    ///
    /// Channel messages currently resolve to the configured default agent.
    pub fn resolve_agent_for_channel(
        &self,
        _channel: &str,
        _account_id: &str,
        _from_id: Option<&str>,
        _is_group: bool,
    ) -> &str {
        &self.default_agent
    }

    // ------------------------------------------------------------------
    // Inbound message routing (thread resolution only, no dispatch)
    // ------------------------------------------------------------------

    /// Resolve the current thread for an inbound message.
    ///
    /// 1. Builds account-scoped user_key from channel/account_id/from_id/is_group/group_id
    /// 2. If user has a switched thread, returns that
    /// 3. Otherwise returns a fresh ordinary thread id
    pub fn resolve_inbound_thread(
        &mut self,
        channel: &str,
        account_id: &str,
        from_id: &str,
        _is_group: bool,
        thread_binding_key: Option<&str>,
    ) -> String {
        let binding_key = thread_binding_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| from_id.trim());
        if binding_key.is_empty() {
            debug!(
                channel,
                account_id, "Resolved inbound thread without binding key -> fresh thread"
            );
            return crate::threads::new_thread_key();
        }
        if let Some(thread_id) =
            self.get_current_thread_id_for_binding(channel, account_id, binding_key)
        {
            info!("Using switched thread: {}", thread_id);
            return thread_id.to_owned();
        }

        debug!(
            "Resolved inbound thread: {}:{} -> fresh thread",
            channel, binding_key
        );
        crate::threads::new_thread_key()
    }

    /// Clone the underlying thread store handle.
    pub fn thread_store(&self) -> Arc<dyn ThreadStore> {
        self.threads.clone()
    }

    // ------------------------------------------------------------------
    // Metadata enrichment
    // ------------------------------------------------------------------

    /// Build a [`MessageMetadata`] from the inbound message context.
    ///
    /// This is called internally by `route_and_dispatch` but is also
    /// available for channel handlers that need the metadata without
    /// full dispatch.
    pub fn enrich_metadata(
        channel: &str,
        account_id: &str,
        from_id: &str,
        is_group: bool,
        thread_id: Option<&str>,
        resolved_thread_id: &str,
    ) -> MessageMetadata {
        MessageMetadata {
            channel: Some(channel.to_owned()),
            account_id: Some(account_id.to_owned()),
            from_id: Some(from_id.to_owned()),
            is_group,
            thread_id: thread_id.map(|s| s.to_owned()),
            resolved_thread_id: Some(resolved_thread_id.to_owned()),
            extra: HashMap::new(),
        }
    }
}
