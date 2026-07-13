use std::collections::{HashMap, VecDeque};

use garyx_models::provider::ProviderRateLimit;
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingAckMarker {
    // Marks the provider echo for the run's initial user message. It must be
    // consumed without acknowledging a queued follow-up.
    RootUserMessage,
    QueuedInput(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PendingAckQueue {
    markers: VecDeque<PendingAckMarker>,
}

impl PendingAckQueue {
    pub(crate) fn with_root_user_message() -> Self {
        Self {
            markers: VecDeque::from([PendingAckMarker::RootUserMessage]),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_root_user_messages(count: usize) -> Self {
        Self {
            markers: std::iter::repeat_n(PendingAckMarker::RootUserMessage, count).collect(),
        }
    }

    pub(crate) fn enqueue(&mut self, pending_input_id: String) {
        self.markers
            .push_back(PendingAckMarker::QueuedInput(pending_input_id));
    }

    pub(crate) fn rollback(&mut self, pending_input_id: &str) {
        if let Some(index) = self.markers.iter().position(
            |marker| matches!(marker, PendingAckMarker::QueuedInput(candidate) if candidate == pending_input_id),
        ) {
            self.markers.remove(index);
        }
    }

    pub(crate) fn acknowledge_next(&mut self, prefer_queued_input: bool) -> Option<String> {
        // Claude can emit assistant/tool activity before echoing the root user
        // message. In that case its next echo belongs to a queued follow-up;
        // Codex passes `false` and consumes the root marker strictly in order.
        if prefer_queued_input
            && matches!(
                self.markers.front(),
                Some(PendingAckMarker::RootUserMessage)
            )
            && self.has_queued_input()
        {
            self.markers.pop_front();
        }
        match self.markers.pop_front() {
            Some(PendingAckMarker::QueuedInput(pending_input_id)) => Some(pending_input_id),
            Some(PendingAckMarker::RootUserMessage) | None => None,
        }
    }

    pub(crate) fn has_queued_input(&self) -> bool {
        self.markers
            .iter()
            .any(|marker| matches!(marker, PendingAckMarker::QueuedInput(_)))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.markers.is_empty()
    }
}

#[derive(Default)]
pub(crate) struct PendingRateLimits {
    by_thread: Mutex<HashMap<String, ProviderRateLimit>>,
}

impl PendingRateLimits {
    pub(crate) async fn clear(&self, thread_id: &str) {
        self.by_thread.lock().await.remove(thread_id);
    }

    pub(crate) async fn stage(&self, thread_id: String, rate_limit: ProviderRateLimit) {
        self.by_thread.lock().await.insert(thread_id, rate_limit);
    }

    pub(crate) async fn take(&self, thread_id: &str) -> Option<ProviderRateLimit> {
        self.by_thread.lock().await.remove(thread_id)
    }
}
