//! Provider quota auto-resend reactor.
//!
//! When a run terminates because the provider's rolling usage quota was
//! exhausted, the bridge commits a `run_complete` control record with
//! `status == "rate_limited"` and a `rate_limit` block carrying the
//! authoritative reset time (sourced from Codex's own
//! `account/rateLimits/updated` snapshot). This reactor watches the committed
//! event stream and, the moment such a record appears, schedules a one-shot
//! `continue` followup for when the window recovers — the exact prompt the
//! rate-limit banner's manual Continue button sends, so the auto and manual
//! recovery paths stay behaviorally identical.
//!
//! Scheduling reuses the file-backed `InternalDispatch` cron primitive so a
//! pending resend survives a gateway restart and is retried on transient
//! dispatch failures. The job is keyed per-thread, so a fresh rate-limit
//! (including one produced by a resend that hit the limit again) replaces any
//! prior pending resend — the thread keeps retrying until it gets through.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use garyx_models::config::{
    CronAction, CronJobConfig, CronJobKind, CronSchedule, InternalDispatchJobPayload,
};
use serde_json::Value;
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::server::AppState;

/// Fire the resend one minute after the reported reset so the provider window
/// has actually rolled over by the time we resubmit.
const RESEND_BUFFER_SECS: i64 = 60;

/// The literal prompt dispatched when the quota window recovers. Matches the
/// rate-limit banner's manual Continue button, which sends the same literal
/// `continue` through the regular send pipeline; the followup metadata block
/// prepended by the cron dispatch path carries the "why" (the auto-resend
/// reason), so the prompt itself stays minimal.
const RESEND_PROMPT: &str = "continue";

/// How many recent events to replay after a broadcast lag, to recover any
/// `rate_limited` record that was dropped from the subscriber buffer. Matches
/// the hub's retained-history depth.
const EVENT_HISTORY_REPLAY: usize = 256;

/// Spawn the background reactor. No-op when no cron service is configured
/// (nothing can be scheduled) or when called outside a Tokio runtime.
pub(crate) fn spawn_reactor(state: Arc<AppState>) {
    if state.ops.cron_service.is_none() {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };

    let mut rx = state.ops.events.subscribe();
    handle.spawn(async move {
        loop {
            match rx.recv().await {
                Ok(raw_event) => {
                    if let Some(plan) = parse_resend_plan(&raw_event) {
                        // Schedule inline rather than in a detached task so
                        // events for the same thread are handled in arrival
                        // order — a newer rate-limit must replace an older
                        // pending resend, and concurrent tasks could finish out
                        // of order and let the stale one win. Scheduling is a
                        // fast history read + cron upsert.
                        schedule_resend(&state, plan).await;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // A burst overran the subscriber buffer and a `rate_limited`
                    // record may have been dropped. Replay recent history so the
                    // resend is still scheduled; per-thread job ids make
                    // re-processing idempotent.
                    for raw_event in state
                        .ops
                        .events
                        .history_snapshot(EVENT_HISTORY_REPLAY)
                        .await
                    {
                        if let Some(plan) = parse_resend_plan(&raw_event) {
                            schedule_resend(&state, plan).await;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResendPlan {
    thread_id: String,
    run_id: String,
    provider: String,
    window: Option<String>,
    reset_at: DateTime<Utc>,
}

/// Parse a committed-event payload into a resend plan, or `None` when the event
/// is not a rate-limited `run_complete` that opted into auto-resend.
fn parse_resend_plan(raw_event: &str) -> Option<ResendPlan> {
    // Cheap prefilter: the vast majority of committed events are not terminal
    // rate-limit records, so avoid a full JSON parse unless the marker is
    // present.
    if !raw_event.contains("rate_limited") {
        return None;
    }

    let event: Value = serde_json::from_str(raw_event).ok()?;
    if event.get("type").and_then(Value::as_str) != Some("committed_message") {
        return None;
    }
    let thread_id = non_empty(event.get("thread_id").and_then(Value::as_str))?;
    let control = event.get("message").and_then(|m| m.get("control"))?;
    if control.get("kind").and_then(Value::as_str) != Some("run_complete") {
        return None;
    }
    if control.get("status").and_then(Value::as_str) != Some("rate_limited") {
        return None;
    }

    let rate_limit = control.get("rate_limit")?;
    if !rate_limit
        .get("will_auto_resend")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let reset_at = rate_limit.get("reset_at").and_then(Value::as_str)?;
    let reset_at = DateTime::parse_from_rfc3339(reset_at)
        .ok()?
        .with_timezone(&Utc);
    let run_id = non_empty(
        control
            .get("run_id")
            .and_then(Value::as_str)
            .or_else(|| event.get("run_id").and_then(Value::as_str)),
    )?;
    let provider = rate_limit
        .get("provider")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("codex")
        .to_owned();
    let window = rate_limit
        .get("window")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned);

    Some(ResendPlan {
        thread_id,
        run_id,
        provider,
        window,
        reset_at,
    })
}

async fn schedule_resend(state: &Arc<AppState>, plan: ResendPlan) {
    let Some(cron) = state.ops.cron_service.as_ref().cloned() else {
        return;
    };

    let now = Utc::now();
    let earliest = now + Duration::seconds(RESEND_BUFFER_SECS);
    let fire_at = (plan.reset_at + Duration::seconds(RESEND_BUFFER_SECS)).max(earliest);
    let delay_seconds = (fire_at - now).num_seconds().max(0) as u64;

    let reason = match &plan.window {
        Some(window) => format!(
            "{} usage-limit auto-resend after {} quota window reset",
            plan.provider, window
        ),
        None => format!(
            "{} usage-limit auto-resend after quota reset",
            plan.provider
        ),
    };

    let cfg = CronJobConfig {
        id: resend_job_id(&plan.thread_id),
        kind: CronJobKind::InternalDispatch {
            payload: InternalDispatchJobPayload {
                prompt: RESEND_PROMPT.to_owned(),
                reason: Some(reason),
                originating_run_id: Some(plan.run_id.clone()),
                scheduled_at: now,
                delay_seconds_requested: delay_seconds,
            },
        },
        label: Some(format!("quota auto-resend ({})", plan.thread_id)),
        schedule: CronSchedule::Once {
            at: fire_at.to_rfc3339(),
        },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: Some(plan.thread_id.clone()),
        delete_after_run: true,
        enabled: true,
        system: true,
    };

    match cron.upsert(cfg).await {
        Ok(_) => info!(
            thread_id = %plan.thread_id,
            run_id = %plan.run_id,
            provider = %plan.provider,
            window = ?plan.window,
            fire_at = %fire_at.to_rfc3339(),
            "scheduled quota auto-resend"
        ),
        Err(error) => warn!(
            thread_id = %plan.thread_id,
            error = %error,
            "failed to schedule quota auto-resend"
        ),
    }
}

/// Deterministic, filesystem-safe, per-thread job id so a newer rate-limit
/// replaces any prior pending resend for the same thread.
fn resend_job_id(thread_id: &str) -> String {
    let sanitized: String = thread_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("quota-resend:{sanitized}")
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use garyx_bridge::{BridgeError, MultiProviderBridge, ProviderRuntime};
    use garyx_models::provider::{
        AgentDispatchOutcome, ProviderRunOptions, ProviderRunResult, ProviderType, QueuedUserInput,
        StreamBoundaryKind, StreamEvent,
    };
    use serde_json::json;
    use std::collections::HashMap;
    use tokio::sync::{Mutex, Notify, mpsc};

    struct QuotaBusyProvider {
        queued_tx: mpsc::UnboundedSender<QueuedUserInput>,
        queued_rx: Mutex<Option<mpsc::UnboundedReceiver<QueuedUserInput>>>,
        active_started: Notify,
        queued_received: Notify,
        allow_ack: Notify,
        release_run: Notify,
    }

    impl QuotaBusyProvider {
        fn new() -> Self {
            let (queued_tx, queued_rx) = mpsc::unbounded_channel();
            Self {
                queued_tx,
                queued_rx: Mutex::new(Some(queued_rx)),
                active_started: Notify::new(),
                queued_received: Notify::new(),
                allow_ack: Notify::new(),
                release_run: Notify::new(),
            }
        }
    }

    #[async_trait]
    impl ProviderRuntime for QuotaBusyProvider {
        fn provider_type(&self) -> ProviderType {
            ProviderType::ClaudeCode
        }

        fn is_ready(&self) -> bool {
            true
        }

        async fn initialize(&mut self) -> Result<(), BridgeError> {
            Ok(())
        }

        async fn shutdown(&mut self) -> Result<(), BridgeError> {
            Ok(())
        }

        async fn run_streaming(
            &self,
            options: &ProviderRunOptions,
            on_chunk: garyx_bridge::provider_trait::StreamCallback,
        ) -> Result<ProviderRunResult, BridgeError> {
            on_chunk(StreamEvent::Delta {
                text: "active reply".to_owned(),
            });
            self.active_started.notify_one();

            let queued = {
                let mut receiver = self.queued_rx.lock().await;
                receiver
                    .as_mut()
                    .expect("quota busy provider receiver is single-use")
                    .recv()
                    .await
                    .expect("quota followup should reach active provider")
            };
            self.queued_received.notify_one();
            self.allow_ack.notified().await;
            on_chunk(StreamEvent::Boundary {
                kind: StreamBoundaryKind::UserAck,
                pending_input_id: queued.pending_input_id,
            });
            self.release_run.notified().await;
            on_chunk(StreamEvent::Done);

            Ok(ProviderRunResult {
                run_id: "quota-busy-provider".to_owned(),
                thread_id: options.thread_id.clone(),
                response: "active reply".to_owned(),
                session_messages: Vec::new(),
                sdk_session_id: Some(format!("sdk-{}", options.thread_id)),
                actual_model: None,
                thread_title: None,
                success: true,
                error: None,
                input_tokens: 0,
                output_tokens: 0,
                cost: 0.0,
                duration_ms: 1,
            })
        }

        async fn add_streaming_input(&self, _thread_id: &str, input: QueuedUserInput) -> bool {
            self.queued_tx.send(input).is_ok()
        }

        async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
            Ok(format!("sdk-{session_key}"))
        }
    }

    fn committed_run_complete(rate_limit: Value) -> String {
        json!({
            "type": "committed_message",
            "thread_id": "thread::abc",
            "run_id": "run::xyz",
            "seq": 7,
            "message": {
                "role": "system",
                "kind": "control",
                "internal": true,
                "internal_kind": "control",
                "control": {
                    "kind": "run_complete",
                    "thread_id": "thread::abc",
                    "run_id": "run::xyz",
                    "status": "rate_limited",
                    "rate_limit": rate_limit,
                }
            }
        })
        .to_string()
    }

    #[test]
    fn parses_rate_limited_run_complete_into_plan() {
        let raw = committed_run_complete(json!({
            "provider": "codex_app_server",
            "window": "primary",
            "reset_at": "2030-01-01T06:00:00+00:00",
            "will_auto_resend": true
        }));
        let plan = parse_resend_plan(&raw).expect("plan parsed");
        assert_eq!(plan.thread_id, "thread::abc");
        assert_eq!(plan.run_id, "run::xyz");
        assert_eq!(plan.provider, "codex_app_server");
        assert_eq!(plan.window.as_deref(), Some("primary"));
        assert_eq!(plan.reset_at.to_rfc3339(), "2030-01-01T06:00:00+00:00");
    }

    #[test]
    fn ignores_when_auto_resend_disabled_or_no_reset() {
        let no_resend = committed_run_complete(json!({
            "provider": "codex",
            "reset_at": "2030-01-01T06:00:00+00:00",
            "will_auto_resend": false
        }));
        assert!(parse_resend_plan(&no_resend).is_none());

        let no_reset = committed_run_complete(json!({
            "provider": "codex",
            "will_auto_resend": true
        }));
        assert!(parse_resend_plan(&no_reset).is_none());
    }

    #[test]
    fn ignores_non_rate_limited_and_non_committed_events() {
        let normal = json!({
            "type": "committed_message",
            "thread_id": "thread::abc",
            "message": { "control": { "kind": "run_complete", "status": "completed" } }
        })
        .to_string();
        assert!(parse_resend_plan(&normal).is_none());
        assert!(parse_resend_plan("not json").is_none());
    }

    #[test]
    fn resend_job_id_is_filesystem_safe_and_thread_scoped() {
        assert_eq!(
            resend_job_id("thread::abc/def"),
            "quota-resend:thread__abc_def"
        );
    }

    /// The scheduled followup dispatches the literal `continue` prompt — the
    /// same prompt the rate-limit banner's manual Continue button sends — and
    /// never a copy of the thread's prior user message.
    #[tokio::test]
    async fn schedules_literal_continue_followup() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cron = Arc::new(crate::automation::CronService::new(
            tmp.path().to_path_buf(),
        ));
        let state = crate::composition::app_bootstrap::AppStateBuilder::new(
            garyx_models::config::GaryxConfig::default(),
        )
        .with_cron_service(cron.clone())
        .build();

        schedule_resend(
            &state,
            ResendPlan {
                thread_id: "thread::abc".to_owned(),
                run_id: "run::xyz".to_owned(),
                provider: "claude".to_owned(),
                window: Some("weekly".to_owned()),
                reset_at: Utc::now() + Duration::seconds(300),
            },
        )
        .await;

        let job = cron
            .get("quota-resend:thread__abc")
            .await
            .expect("resend job scheduled");
        let CronJobKind::InternalDispatch { payload } = &job.kind else {
            panic!("expected internal-dispatch job, got {:?}", job.kind);
        };
        assert_eq!(payload.prompt, "continue");
        assert_eq!(payload.originating_run_id.as_deref(), Some("run::xyz"));
        assert!(
            payload
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("claude usage-limit auto-resend"),
            "reason should carry the auto-resend context: {:?}",
            payload.reason
        );
        assert_eq!(job.thread_id.as_deref(), Some("thread::abc"));
        assert!(job.delete_after_run);
    }

    #[tokio::test]
    async fn quota_auto_resend_preserves_followup_metadata_through_busy_queue_ack() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cron = Arc::new(crate::automation::CronService::new(
            tmp.path().to_path_buf(),
        ));
        let bridge = Arc::new(MultiProviderBridge::new());
        let provider = Arc::new(QuotaBusyProvider::new());
        bridge
            .register_provider("quota-busy-provider", provider.clone())
            .await;
        bridge
            .set_route("telegram", "bot1", "quota-busy-provider")
            .await;
        bridge.set_default_provider_key("quota-busy-provider").await;

        let state = crate::composition::app_bootstrap::AppStateBuilder::new(
            garyx_models::config::GaryxConfig::default(),
        )
        .with_bridge(bridge.clone())
        .with_cron_service(cron.clone())
        .with_custom_agent_store(Arc::new(crate::custom_agents::CustomAgentStore::new()))
        .build();
        bridge
            .set_thread_store(state.threads.thread_store.clone())
            .await;
        bridge.set_event_tx(state.ops.events.sender()).await;

        let thread_id = "thread::quota-resend-busy";
        state
            .threads
            .thread_store
            .set(
                thread_id,
                json!({
                    "thread_id": thread_id,
                    "channel": "telegram",
                    "account_id": "bot1",
                    "from_id": "test-user",
                    "is_group": false,
                    "messages": [],
                    "channel_bindings": [{
                        "channel": "telegram",
                        "account_id": "bot1",
                        "binding_key": "test-user",
                        "chat_id": "test-user",
                        "display_label": "Test User"
                    }],
                    "delivery_context": {
                        "channel": "telegram",
                        "account_id": "bot1",
                        "chat_id": "test-user",
                        "user_id": "test-user",
                        "delivery_target_type": "chat_id",
                        "delivery_target_id": "test-user",
                        "thread_id": "test-user",
                        "metadata": {}
                    }
                }),
            )
            .await
            .unwrap();

        let active_outcome = crate::internal_inbound::dispatch_internal_message_to_thread(
            &state,
            thread_id,
            "run::active",
            "active turn",
            crate::internal_inbound::InternalDispatchOptions::default(),
        )
        .await
        .expect("active turn should start through the production front door");
        assert_eq!(active_outcome, AgentDispatchOutcome::Started);
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            provider.active_started.notified(),
        )
        .await
        .expect("active provider run should become busy");

        spawn_reactor(state.clone());
        let rate_limited_run_id = "run::rate-limited";
        let event = committed_run_complete(json!({
            "provider": "claude",
            "window": "weekly",
            "reset_at": (Utc::now() + Duration::minutes(5)).to_rfc3339(),
            "will_auto_resend": true
        }))
        .replace("run::xyz", rate_limited_run_id)
        .replace("thread::abc", thread_id);
        state
            .ops
            .events
            .sender()
            .send(event)
            .expect("quota reactor should be subscribed");

        let job_id = resend_job_id(thread_id);
        let job = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if let Some(job) = cron.get(&job_id).await {
                    break job;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("real quota reactor should schedule its internal-dispatch job");
        let CronJobKind::InternalDispatch { payload } = &job.kind else {
            panic!(
                "quota reactor scheduled unexpected job kind: {:?}",
                job.kind
            );
        };
        let expected = HashMap::from([
            ("schedule_followup_job_id", Value::String(job_id.clone())),
            (
                "schedule_followup_scheduled_at",
                Value::String(payload.scheduled_at.to_rfc3339()),
            ),
            (
                "schedule_followup_scheduled_for",
                Value::String(job.next_run.to_rfc3339()),
            ),
            (
                "schedule_followup_reason",
                Value::String(
                    payload
                        .reason
                        .clone()
                        .expect("quota resend reason is required"),
                ),
            ),
            (
                "schedule_followup_originating_run_id",
                Value::String(rate_limited_run_id.to_owned()),
            ),
        ]);

        cron.run_now(
            &job_id,
            &crate::composition::automation_wiring::automation_exec_env(&state),
        )
        .await
        .expect("quota resend should dispatch into the busy thread");
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            provider.queued_received.notified(),
        )
        .await
        .expect("active provider should receive the quota resend");

        let pending = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let thread = state
                    .threads
                    .thread_store
                    .get(thread_id)
                    .await
                    .unwrap()
                    .expect("thread should remain present");
                if let Some(pending) = thread["pending_user_inputs"].as_array().and_then(|items| {
                    items.iter().find(|item| {
                        item.pointer("/metadata/schedule_followup_job_id")
                            == Some(&Value::String(job_id.clone()))
                    })
                }) {
                    break pending.clone();
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("quota resend should be persisted pending before ACK");
        for (key, value) in &expected {
            assert_eq!(
                pending.pointer(&format!("/metadata/{key}")),
                Some(value),
                "pending quota resend lost {key}: {pending}"
            );
        }

        provider.allow_ack.notify_one();
        let committed = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let snapshot = state
                    .threads
                    .history
                    .thread_snapshot(thread_id, 100)
                    .await
                    .expect("thread history should load");
                if let Some(message) = snapshot.committed_messages.iter().find(|message| {
                    message.pointer("/metadata/schedule_followup_job_id")
                        == Some(&Value::String(job_id.clone()))
                }) {
                    break message.clone();
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("provider ACK should commit the quota resend user record");
        for (key, value) in &expected {
            assert_eq!(
                committed.pointer(&format!("/metadata/{key}")),
                Some(value),
                "committed quota resend lost {key}: {committed}"
            );
        }

        provider.release_run.notify_one();
    }
}
