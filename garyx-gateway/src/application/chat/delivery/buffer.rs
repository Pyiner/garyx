use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use garyx_models::ChannelOutboundContent;

use super::plan::BoundThreadDeliveryTarget;
use super::sender::{
    deliver_assistant_reply_to_bound_channels, deliver_markdown_images_to_bound_channels,
    deliver_structured_content_to_bound_channels,
};
use crate::server::AppState;

#[cfg(test)]
pub(crate) const LOOP_BOUND_DELIVERY_FLUSH_DELAY: Duration = Duration::from_millis(20);
#[cfg(not(test))]
pub(crate) const LOOP_BOUND_DELIVERY_FLUSH_DELAY: Duration = Duration::from_secs(10);

#[derive(Clone, Default)]
pub(crate) struct BoundThreadDeliveryBuffer {
    pending: Arc<std::sync::Mutex<String>>,
    image_scan: Arc<std::sync::Mutex<String>>,
    targets: Arc<Vec<BoundThreadDeliveryTarget>>,
    suppressed: Arc<AtomicBool>,
    delivery_gate: Arc<tokio::sync::Mutex<()>>,
    inflight: Arc<std::sync::atomic::AtomicUsize>,
    idle_notify: Arc<tokio::sync::Notify>,
}

impl BoundThreadDeliveryBuffer {
    pub(super) fn with_targets(targets: Vec<BoundThreadDeliveryTarget>) -> Self {
        Self {
            targets: Arc::new(targets),
            ..Default::default()
        }
    }

    fn targets_snapshot(&self) -> Vec<BoundThreadDeliveryTarget> {
        (*self.targets).clone()
    }

    pub(crate) fn push_delta(&self, text: &str, warn_context: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        self.suppressed.store(false, Ordering::Relaxed);
        if let Ok(mut pending) = self.pending.lock() {
            let was_empty = pending.is_empty();
            pending.push_str(text);
            if let Ok(mut image_scan) = self.image_scan.lock() {
                image_scan.push_str(text);
            } else {
                tracing::warn!(
                    "{warn_context}: image scan lock poisoned while collecting assistant delta"
                );
            }
            was_empty
        } else {
            tracing::warn!("{warn_context}: buffer lock poisoned while collecting assistant delta");
            false
        }
    }

    pub(crate) fn push_image_scan_delta(&self, text: &str, warn_context: &str) {
        if text.is_empty() {
            return;
        }
        if let Ok(mut image_scan) = self.image_scan.lock() {
            image_scan.push_str(text);
        } else {
            tracing::warn!(
                "{warn_context}: image scan lock poisoned while collecting streaming assistant delta"
            );
        }
    }

    pub(crate) fn suppress(&self) {
        let should_suppress = match self.pending.lock() {
            Ok(pending) => pending.trim().is_empty(),
            Err(_) => {
                tracing::warn!(
                    "bound delivery buffer lock poisoned while deciding message-tool suppression"
                );
                true
            }
        };
        if should_suppress {
            self.suppressed.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn push_separator(&self, warn_context: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            if !pending.trim().is_empty() && !pending.ends_with("\n\n") {
                if pending.ends_with('\n') {
                    pending.push('\n');
                } else {
                    pending.push_str("\n\n");
                }
            }
        } else {
            tracing::warn!(
                "{warn_context}: buffer lock poisoned while collecting assistant boundary"
            );
        }
        self.push_image_scan_separator(warn_context);
    }

    pub(crate) fn push_image_scan_separator(&self, warn_context: &str) {
        if let Ok(mut image_scan) = self.image_scan.lock() {
            if image_scan.trim().is_empty() || image_scan.ends_with("\n\n") {
                return;
            }
            if image_scan.ends_with('\n') {
                image_scan.push('\n');
            } else {
                image_scan.push_str("\n\n");
            }
        } else {
            tracing::warn!(
                "{warn_context}: image scan lock poisoned while collecting assistant boundary"
            );
        }
    }

    pub(super) fn take_pending_text(&self, warn_context: &'static str) -> Option<String> {
        if self.suppressed.load(Ordering::Relaxed) {
            return None;
        }

        let merged = match self.pending.lock() {
            Ok(mut pending) => std::mem::take(&mut *pending),
            Err(_) => {
                tracing::warn!(
                    "{warn_context}: buffer lock poisoned while finalizing assistant delivery"
                );
                return None;
            }
        };
        (!merged.trim().is_empty()).then_some(merged)
    }

    pub(super) fn take_image_scan_text(&self, warn_context: &'static str) -> Option<String> {
        if self.suppressed.load(Ordering::Relaxed) {
            return None;
        }

        let merged = match self.image_scan.lock() {
            Ok(mut pending) => std::mem::take(&mut *pending),
            Err(_) => {
                tracing::warn!(
                    "{warn_context}: image scan lock poisoned while finalizing assistant delivery"
                );
                return None;
            }
        };
        (!merged.trim().is_empty()).then_some(merged)
    }

    pub(crate) fn flush(
        &self,
        state: Arc<AppState>,
        thread_id: String,
        run_id: String,
        warn_context: &'static str,
    ) {
        let Some(merged) = self.take_pending_text(warn_context) else {
            return;
        };

        let delivery_gate = self.delivery_gate.clone();
        let inflight = self.inflight.clone();
        let idle_notify = self.idle_notify.clone();
        let targets = self.targets_snapshot();
        inflight.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            let _guard = delivery_gate.lock().await;
            deliver_assistant_reply_to_bound_channels(state, thread_id, run_id, merged, targets)
                .await;
            if inflight.fetch_sub(1, Ordering::Relaxed) == 1 {
                idle_notify.notify_waiters();
            }
        });
    }

    pub(crate) fn dispatch_content_after_flush(
        &self,
        state: Arc<AppState>,
        thread_id: String,
        run_id: String,
        content: ChannelOutboundContent,
        warn_context: &'static str,
    ) {
        let pending_text = self.take_pending_text(warn_context);
        let delivery_gate = self.delivery_gate.clone();
        let inflight = self.inflight.clone();
        let idle_notify = self.idle_notify.clone();
        let targets = self.targets_snapshot();
        inflight.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            let _guard = delivery_gate.lock().await;
            if let Some(text) = pending_text {
                deliver_assistant_reply_to_bound_channels(
                    state.clone(),
                    thread_id.clone(),
                    run_id.clone(),
                    text,
                    targets.clone(),
                )
                .await;
            }
            deliver_structured_content_to_bound_channels(
                state, thread_id, run_id, content, targets,
            )
            .await;
            if inflight.fetch_sub(1, Ordering::Relaxed) == 1 {
                idle_notify.notify_waiters();
            }
        });
    }

    pub(crate) fn finish(
        &self,
        state: Arc<AppState>,
        thread_id: String,
        run_id: String,
        warn_context: &'static str,
    ) {
        let pending_text = self.take_pending_text(warn_context);
        let image_scan_text = self.take_image_scan_text(warn_context);
        if pending_text.is_none() && image_scan_text.is_none() {
            return;
        }

        let delivery_gate = self.delivery_gate.clone();
        let inflight = self.inflight.clone();
        let idle_notify = self.idle_notify.clone();
        let targets = self.targets_snapshot();
        inflight.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            let _guard = delivery_gate.lock().await;
            if let Some(text) = pending_text {
                deliver_assistant_reply_to_bound_channels(
                    state.clone(),
                    thread_id.clone(),
                    run_id.clone(),
                    text,
                    targets.clone(),
                )
                .await;
            }
            if let Some(text) = image_scan_text {
                deliver_markdown_images_to_bound_channels(state, thread_id, run_id, &text, targets)
                    .await;
            }
            if inflight.fetch_sub(1, Ordering::Relaxed) == 1 {
                idle_notify.notify_waiters();
            }
        });
    }

    pub(crate) fn finish_markdown_images_after(
        &self,
        state: Arc<AppState>,
        thread_id: String,
        run_id: String,
        warn_context: &'static str,
        delay: Duration,
    ) {
        let image_scan_text = self.take_image_scan_text(warn_context);
        let Some(text) = image_scan_text else {
            return;
        };

        let delivery_gate = self.delivery_gate.clone();
        let inflight = self.inflight.clone();
        let idle_notify = self.idle_notify.clone();
        let targets = self.targets_snapshot();
        inflight.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let _guard = delivery_gate.lock().await;
            deliver_markdown_images_to_bound_channels(state, thread_id, run_id, &text, targets)
                .await;
            if inflight.fetch_sub(1, Ordering::Relaxed) == 1 {
                idle_notify.notify_waiters();
            }
        });
    }
}

pub(super) fn schedule_loop_bound_delivery_flush(
    buffer: BoundThreadDeliveryBuffer,
    scheduled: Arc<AtomicBool>,
    state: Arc<AppState>,
    thread_id: String,
    run_id: String,
) {
    if scheduled.swap(true, Ordering::Relaxed) {
        return;
    }

    tokio::spawn(async move {
        tokio::time::sleep(LOOP_BOUND_DELIVERY_FLUSH_DELAY).await;
        scheduled.store(false, Ordering::Relaxed);
        buffer.flush(state, thread_id, run_id, "loop bound delivery");
    });
}
