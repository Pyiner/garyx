use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Local, NaiveTime, Timelike, Utc};
use chrono_tz::Tz;
use garyx_bridge::MultiProviderBridge;
use garyx_channels::{ChannelDispatcher, OutboundMessage, SendMessageResult};
use garyx_models::config::{HeartbeatConfig, McpServerConfig};
use garyx_models::provider::{AgentRunRequest, StreamBoundaryKind, StreamEvent};
use garyx_models::thread_logs::{ThreadLogEvent, ThreadLogSink};
use garyx_router::MessageRouter;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex, RwLock, broadcast, mpsc};
use uuid::Uuid;

use crate::delivery_target::resolve_delivery_target_with_recovery;
use crate::managed_mcp_metadata::inject_managed_mcp_servers;
use crate::skills::sync_default_external_user_skills;

// ---------------------------------------------------------------------------
// HeartbeatService
// ---------------------------------------------------------------------------

/// Record of a single heartbeat event.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HeartbeatRecord {
    pub timestamp: DateTime<Utc>,
    pub target: String,
    pub skipped: bool,
    #[serde(default)]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Maximum number of recent heartbeat records to keep.
const MAX_RECENT_RECORDS: usize = 100;

#[derive(Clone)]
struct HeartbeatDispatchRuntime {
    router: Arc<Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
    channel_dispatcher: Arc<dyn ChannelDispatcher>,
    thread_logs: Arc<dyn ThreadLogSink>,
    managed_mcp_servers: HashMap<String, McpServerConfig>,
}

/// Background service that fires periodic heartbeat events.
///
/// Respects `enabled`, `every` (interval), and optional `active_hours` from config.
/// Lifecycle: `start()` spawns a tokio task, `stop()` terminates it.
pub struct HeartbeatService {
    config: HeartbeatConfig,
    stop_tx: Option<mpsc::Sender<()>>,
    event_tx: Option<broadcast::Sender<String>>,
    recent: Arc<RwLock<VecDeque<HeartbeatRecord>>>,
    data_dir: Option<PathBuf>,
    dispatch_runtime: Arc<RwLock<Option<HeartbeatDispatchRuntime>>>,
}

impl HeartbeatService {
    pub fn new(config: HeartbeatConfig) -> Self {
        Self {
            config,
            stop_tx: None,
            event_tx: None,
            recent: Arc::new(RwLock::new(VecDeque::new())),
            data_dir: None,
            dispatch_runtime: Arc::new(RwLock::new(None)),
        }
    }

    /// Attach a broadcast channel for publishing heartbeat events.
    pub fn set_event_tx(&mut self, tx: broadcast::Sender<String>) {
        self.event_tx = Some(tx);
    }

    /// Configure persistence root dir for heartbeat records.
    pub fn set_data_dir(&mut self, data_dir: PathBuf) {
        self.data_dir = Some(data_dir);
    }

    /// Attach bridge+router runtime for delivery-context dispatch.
    pub async fn set_dispatch_runtime(
        &self,
        router: Arc<Mutex<MessageRouter>>,
        bridge: Arc<MultiProviderBridge>,
        channel_dispatcher: Arc<dyn ChannelDispatcher>,
        thread_logs: Arc<dyn ThreadLogSink>,
        managed_mcp_servers: HashMap<String, McpServerConfig>,
    ) {
        *self.dispatch_runtime.write().await = Some(HeartbeatDispatchRuntime {
            router,
            bridge,
            channel_dispatcher,
            thread_logs,
            managed_mcp_servers,
        });
    }

    /// Load persisted heartbeat records from disk (best-effort).
    pub async fn load_persisted_records(&self) -> std::io::Result<()> {
        let Some(data_dir) = &self.data_dir else {
            return Ok(());
        };
        let records = load_records(data_dir).await?;
        let mut guard = self.recent.write().await;
        *guard = records;
        Ok(())
    }

    /// Parse the `every` field into a `Duration`.
    ///
    /// Python parity:
    /// - Supports `"500ms"`, `"30s"`, `"5m"`, `"3h"`, `"1d"`, decimal values like `"0.5s"`.
    /// - If unit is omitted, defaults to minutes (e.g. `"30"` => 30 minutes).
    fn parse_interval(every: &str) -> std::time::Duration {
        let compact: String = every.chars().filter(|c| !c.is_whitespace()).collect();
        let s = compact.trim().to_ascii_lowercase();
        if s.is_empty() {
            return std::time::Duration::from_secs(3 * 3600);
        }

        let (value_part, unit) = if let Some(n) = s.strip_suffix("ms") {
            (n, "ms")
        } else if let Some(n) = s.strip_suffix('s') {
            (n, "s")
        } else if let Some(n) = s.strip_suffix('m') {
            (n, "m")
        } else if let Some(n) = s.strip_suffix('h') {
            (n, "h")
        } else if let Some(n) = s.strip_suffix('d') {
            (n, "d")
        } else {
            // Python parity: default unit is minutes.
            (s.as_str(), "m")
        };

        let value = match value_part.parse::<f64>() {
            Ok(v) if v.is_finite() && v >= 0.0 => v,
            _ => return std::time::Duration::from_secs(3 * 3600),
        };

        let multiplier_ms = match unit {
            "ms" => 1.0,
            "s" => 1_000.0,
            "m" => 60_000.0,
            "h" => 3_600_000.0,
            "d" => 86_400_000.0,
            _ => return std::time::Duration::from_secs(3 * 3600),
        };

        let total_ms = value * multiplier_ms;
        if !total_ms.is_finite() || total_ms < 0.0 || total_ms > (u64::MAX as f64) {
            return std::time::Duration::from_secs(3 * 3600);
        }

        std::time::Duration::from_millis(total_ms as u64)
    }

    /// Check whether the current time is within active hours.
    ///
    /// If no active_hours are configured, always returns true.
    fn is_within_active_hours(config: &HeartbeatConfig) -> bool {
        Self::is_within_active_hours_at(config, Utc::now())
    }

    fn is_within_active_hours_at(config: &HeartbeatConfig, now_utc: DateTime<Utc>) -> bool {
        let active = match &config.active_hours {
            Some(ah) => ah,
            None => return true,
        };

        let Some(start_min) = parse_active_hours_minutes(&active.start, false) else {
            return true;
        };
        let Some(end_min) = parse_active_hours_minutes(&active.end, true) else {
            return true;
        };
        if start_min == end_min {
            return true;
        }
        let Some(current_min) = resolve_current_minutes(now_utc, &active.timezone) else {
            return true;
        };

        if end_min > start_min {
            current_min >= start_min && current_min < end_min
        } else {
            // Wraps midnight (e.g. 22:00 - 06:00).
            current_min >= start_min || current_min < end_min
        }
    }

    /// Start the heartbeat background loop.
    pub fn start(&mut self) {
        if !self.config.enabled {
            tracing::info!("heartbeat service disabled by config");
            return;
        }
        if self.stop_tx.is_some() {
            tracing::warn!("heartbeat service already running; duplicate start ignored");
            return;
        }

        let interval = Self::parse_interval(&self.config.every);
        tracing::info!(
            interval_secs = interval.as_secs(),
            "heartbeat service starting"
        );

        let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
        self.stop_tx = Some(stop_tx);

        let config = self.config.clone();
        let event_tx = self.event_tx.clone();
        let recent = self.recent.clone();
        let data_dir = self.data_dir.clone();
        let dispatch_runtime = self.dispatch_runtime.clone();

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // Skip the initial immediate tick.
            ticker.tick().await;
            let startup_delay = tokio::time::Duration::from_secs(2);
            let startup_timer = tokio::time::sleep(startup_delay);
            tokio::pin!(startup_timer);
            let mut startup_fired = false;

            loop {
                tokio::select! {
                    _ = stop_rx.recv() => {
                        tracing::info!("heartbeat service stopping");
                        break;
                    }
                    _ = &mut startup_timer, if !startup_fired => {
                        startup_fired = true;
                        Self::fire_tracked(
                            &config,
                            event_tx.as_ref(),
                            &recent,
                            data_dir.as_deref(),
                            &dispatch_runtime,
                        ).await;
                    }
                    _ = ticker.tick() => {
                        Self::fire_tracked(
                            &config,
                            event_tx.as_ref(),
                            &recent,
                            data_dir.as_deref(),
                            &dispatch_runtime,
                        ).await;
                    }
                }
            }
            tracing::info!("heartbeat service stopped");
        });
    }

    /// Stop the heartbeat loop.
    pub async fn stop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(()).await;
        }
    }

    /// Manually trigger a heartbeat (AC-3).
    pub async fn trigger(&self) {
        Self::fire_tracked(
            &self.config,
            self.event_tx.as_ref(),
            &self.recent,
            self.data_dir.as_deref(),
            &self.dispatch_runtime,
        )
        .await;
    }

    /// Return recent heartbeat records.
    pub async fn recent_records(&self) -> Vec<HeartbeatRecord> {
        self.recent.read().await.iter().cloned().collect()
    }

    /// Return the heartbeat config.
    pub fn config(&self) -> &HeartbeatConfig {
        &self.config
    }

    /// Fire a single heartbeat tick with tracking.
    async fn fire_tracked(
        config: &HeartbeatConfig,
        event_tx: Option<&broadcast::Sender<String>>,
        recent: &Arc<RwLock<VecDeque<HeartbeatRecord>>>,
        data_dir: Option<&Path>,
        dispatch_runtime: &Arc<RwLock<Option<HeartbeatDispatchRuntime>>>,
    ) {
        let timestamp = Utc::now();
        let skipped = !Self::is_within_active_hours(config);
        let mut status = if skipped { "skipped" } else { "fired" }.to_owned();
        let mut run_id = None;
        let mut thread_id = None;
        let mut channel = None;
        let mut account_id = None;
        let mut error = None;

        if skipped {
            tracing::debug!("heartbeat skipped: outside active hours");
        } else {
            tracing::info!(target = %config.target, "heartbeat fired");

            if let Some(runtime) = dispatch_runtime.read().await.clone() {
                match dispatch_heartbeat(config, &runtime).await {
                    Ok((rid, tid, ch, aid)) => {
                        status = "dispatched".to_owned();
                        run_id = Some(rid);
                        thread_id = tid;
                        channel = Some(ch);
                        account_id = Some(aid);
                    }
                    Err(e) => {
                        status = "dispatch_failed".to_owned();
                        error = Some(e.clone());
                        tracing::warn!(error = %e, "heartbeat dispatch failed");
                    }
                }
            }

            // Publish event for SSE / observability.
            if let Some(tx) = event_tx {
                let event = serde_json::json!({
                    "type": "heartbeat_fired",
                    "target": config.target,
                    "status": status,
                    "run_id": run_id,
                    "thread_id": thread_id,
                    "channel": channel,
                    "account_id": account_id,
                    "error": error,
                    "timestamp": timestamp.to_rfc3339(),
                });
                let _ = tx.send(event.to_string());
            }
        }

        // Track the record.
        let record = HeartbeatRecord {
            timestamp,
            target: config.target.clone(),
            skipped,
            status,
            run_id,
            thread_id,
            channel,
            account_id,
            error,
        };
        let mut recent_guard = recent.write().await;
        recent_guard.push_back(record);
        while recent_guard.len() > MAX_RECENT_RECORDS {
            recent_guard.pop_front();
        }

        if let Some(root) = data_dir {
            let _ = persist_records(root, &recent_guard).await;
        }

        // Clean up stale heartbeat thread files older than 1 day.
        if let Some(runtime) = dispatch_runtime.read().await.as_ref() {
            let store = runtime.router.lock().await.thread_store();
            let keys = store.list_keys(Some("default::heartbeat::")).await;
            let one_day_ago = Utc::now() - chrono::Duration::days(1);
            let mut cleaned = 0usize;
            for key in keys {
                if let Some(data) = store.get(&key).await {
                    let is_stale = data
                        .get("_updated_at")
                        .or_else(|| data.get("updated_at"))
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                        .map(|t| t < one_day_ago)
                        .unwrap_or(true); // no timestamp = stale
                    if is_stale {
                        let provider_key = data
                            .get("provider_key")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned);
                        let has_affinity = runtime.bridge.thread_affinity_for(&key).await.is_some();
                        let clear_succeeded = if provider_key.is_some() || has_affinity {
                            runtime
                                .bridge
                                .clear_thread_state(&key, provider_key.as_deref())
                                .await
                        } else {
                            true
                        };
                        if clear_succeeded && store.delete(&key).await {
                            cleaned += 1;
                        }
                    }
                }
            }
            if cleaned > 0 {
                tracing::info!(
                    removed = cleaned,
                    "Cleaned up stale heartbeat threads (>1 day old)"
                );
            }
        }
    }
}

fn resolve_current_minutes(now_utc: DateTime<Utc>, timezone: &str) -> Option<i32> {
    let tz_trim = timezone.trim();
    let tz_lower = tz_trim.to_ascii_lowercase();

    let naive = if tz_lower == "local" || tz_lower == "user" {
        let local_now = now_utc.with_timezone(&Local);
        NaiveTime::from_hms_opt(local_now.hour(), local_now.minute(), 0)
    } else if tz_lower == "utc" || tz_trim.is_empty() {
        NaiveTime::from_hms_opt(now_utc.hour(), now_utc.minute(), 0)
    } else if let Ok(tz) = tz_trim.parse::<Tz>() {
        let tz_now = now_utc.with_timezone(&tz);
        NaiveTime::from_hms_opt(tz_now.hour(), tz_now.minute(), 0)
    } else {
        NaiveTime::from_hms_opt(now_utc.hour(), now_utc.minute(), 0)
    };

    let current = naive?;
    Some((current.hour() as i32 * 60) + current.minute() as i32)
}

fn parse_active_hours_minutes(input: &str, allow_24: bool) -> Option<i32> {
    let value = input.trim();
    let (hour_str, minute_str) = value.split_once(':')?;
    let hour = hour_str.parse::<i32>().ok()?;
    let minute = minute_str.parse::<i32>().ok()?;
    if minute < 0 || minute >= 60 {
        return None;
    }
    if hour == 24 {
        if allow_24 && minute == 0 {
            return Some(24 * 60);
        }
        return None;
    }
    if (0..24).contains(&hour) {
        return Some(hour * 60 + minute);
    }
    None
}

fn scheduled_thread_log_id(thread_key: &str, thread_id: Option<&str>) -> Option<String> {
    thread_id
        .map(str::trim)
        .filter(|value| garyx_models::thread_logs::is_canonical_thread_id(value))
        .map(ToOwned::to_owned)
        .or_else(|| {
            let trimmed = thread_key.trim();
            garyx_models::thread_logs::is_canonical_thread_id(trimmed).then(|| trimmed.to_owned())
        })
}

async fn dispatch_heartbeat(
    config: &HeartbeatConfig,
    runtime: &HeartbeatDispatchRuntime,
) -> Result<(String, Option<String>, String, String), String> {
    let (base_thread_id, delivery) =
        resolve_delivery_target_with_recovery(&runtime.router, &config.target)
            .await
            .ok_or_else(|| format!("no delivery context found for target `{}`", config.target))?;

    let run_id = Uuid::new_v4().to_string();
    let heartbeat_thread_id = format!("{}::heartbeat::{}", base_thread_id, &run_id[..8]);
    let mut metadata = HashMap::new();
    metadata.insert("source".to_owned(), Value::String("heartbeat".to_owned()));
    metadata.insert("target".to_owned(), Value::String(config.target.clone()));
    metadata.insert(
        "base_thread_id".to_owned(),
        Value::String(base_thread_id.clone()),
    );
    metadata.insert(
        "delivery_chat_id".to_owned(),
        Value::String(delivery.chat_id.clone()),
    );
    if let Some(tid) = &delivery.thread_id {
        metadata.insert("thread_id".to_owned(), Value::String(tid.clone()));
    }
    inject_managed_mcp_servers(&runtime.managed_mcp_servers, &mut metadata);
    let thread_log_id = scheduled_thread_log_id(&base_thread_id, delivery.thread_id.as_deref());

    let response_callback = Some(build_scheduled_response_callback(
        runtime.channel_dispatcher.clone(),
        runtime.router.clone(),
        ScheduledResponseContext {
            thread_id: heartbeat_thread_id.clone(),
            channel: delivery.channel.clone(),
            account_id: delivery.account_id.clone(),
            chat_id: delivery.chat_id.clone(),
            delivery_target_type: delivery.delivery_target_type.clone(),
            delivery_target_id: delivery.delivery_target_id.clone(),
            delivery_thread_id: delivery.thread_id.clone(),
            thread_log_id: thread_log_id.clone(),
        },
    ));
    if let Some(thread_id) = &thread_log_id {
        runtime
            .thread_logs
            .record_event(
                ThreadLogEvent::info(thread_id, "heartbeat", "heartbeat dispatch started")
                    .with_run_id(run_id.clone())
                    .with_field("target", Value::String(config.target.clone()))
                    .with_field("channel", Value::String(delivery.channel.clone()))
                    .with_field("account_id", Value::String(delivery.account_id.clone()))
                    .with_field("chat_id", Value::String(delivery.chat_id.clone()))
                    .with_field(
                        "thread_id",
                        Value::String(
                            thread_log_id
                                .clone()
                                .unwrap_or_else(|| heartbeat_thread_id.clone()),
                        ),
                    ),
            )
            .await;
    }

    if let Err(error) = sync_default_external_user_skills() {
        tracing::warn!(
            error = %error,
            thread_id = %heartbeat_thread_id,
            "failed to sync external user skills before heartbeat dispatch"
        );
    }

    if let Err(error) = runtime
        .bridge
        .start_agent_run(
            AgentRunRequest::new(
                &heartbeat_thread_id,
                "Heartbeat check-in",
                &run_id,
                &delivery.channel,
                &delivery.account_id,
                metadata,
            ),
            response_callback,
        )
        .await
    {
        if let Some(thread_id) = &thread_log_id {
            runtime
                .thread_logs
                .record_event(
                    ThreadLogEvent::error(thread_id, "heartbeat", "heartbeat dispatch failed")
                        .with_run_id(run_id.clone())
                        .with_field("target", Value::String(config.target.clone()))
                        .with_field("error", Value::String(error.to_string())),
                )
                .await;
        }
        return Err(format!("bridge dispatch error: {error}"));
    }

    if let Some(thread_id) = &thread_log_id {
        runtime
            .thread_logs
            .record_event(
                ThreadLogEvent::info(thread_id, "heartbeat", "heartbeat dispatch accepted")
                    .with_run_id(run_id.clone())
                    .with_field("target", Value::String(config.target.clone()))
                    .with_field(
                        "thread_id",
                        Value::String(
                            thread_log_id
                                .clone()
                                .unwrap_or_else(|| heartbeat_thread_id.clone()),
                        ),
                    ),
            )
            .await;
    }

    Ok((
        run_id,
        thread_log_id,
        delivery.channel.clone(),
        delivery.account_id.clone(),
    ))
}

fn heartbeat_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("heartbeat").join("records.json")
}

async fn load_records(data_dir: &Path) -> std::io::Result<VecDeque<HeartbeatRecord>> {
    let path = heartbeat_file_path(data_dir);
    if !path.exists() {
        return Ok(VecDeque::new());
    }

    let bytes = tokio::fs::read(path).await?;
    let records: Vec<HeartbeatRecord> = serde_json::from_slice(&bytes).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid records json: {e}"),
        )
    })?;

    let mut deque = VecDeque::from(records);
    while deque.len() > MAX_RECENT_RECORDS {
        deque.pop_front();
    }
    Ok(deque)
}

async fn persist_records(
    data_dir: &Path,
    records: &VecDeque<HeartbeatRecord>,
) -> std::io::Result<()> {
    let dir = data_dir.join("heartbeat");
    tokio::fs::create_dir_all(&dir).await?;

    let path = heartbeat_file_path(data_dir);
    let tmp = path.with_extension("tmp");
    let list: Vec<HeartbeatRecord> = records.iter().cloned().collect();
    let bytes = serde_json::to_vec_pretty(&list)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("{e}")))?;
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(())
}

fn format_scheduled_message(text: &str, thread_id: &str) -> String {
    if text.is_empty() || !MessageRouter::is_scheduled_thread(thread_id) {
        return text.to_owned();
    }

    let header = format!("#{thread_id}");
    if text.trim_start().starts_with(&header) {
        return text.to_owned();
    }

    format!("{header}\n{text}")
}

struct ScheduledResponseContext {
    thread_id: String,
    channel: String,
    account_id: String,
    chat_id: String,
    delivery_target_type: String,
    delivery_target_id: String,
    delivery_thread_id: Option<String>,
    thread_log_id: Option<String>,
}

fn build_scheduled_response_callback(
    dispatcher: Arc<dyn ChannelDispatcher>,
    router: Arc<Mutex<MessageRouter>>,
    context: ScheduledResponseContext,
) -> Arc<dyn Fn(StreamEvent) + Send + Sync> {
    let pending = Arc::new(std::sync::Mutex::new(ScheduledStreamState::default()));
    let ScheduledResponseContext {
        thread_id,
        channel,
        account_id,
        chat_id,
        delivery_target_type,
        delivery_target_id,
        delivery_thread_id,
        thread_log_id,
    } = context;

    Arc::new(move |event: StreamEvent| {
        let maybe_message = {
            let mut buf = match pending.lock() {
                Ok(buf) => buf,
                Err(_) => {
                    tracing::warn!("scheduled response callback buffer lock poisoned");
                    return;
                }
            };
            match event {
                StreamEvent::Delta { text } => {
                    if !buf.closed_after_user_ack && !text.is_empty() {
                        buf.text.push_str(&text);
                    }
                    None
                }
                StreamEvent::Boundary {
                    kind: StreamBoundaryKind::AssistantSegment,
                    ..
                } => {
                    if !buf.closed_after_user_ack {
                        append_inline_assistant_separator(&mut buf.text);
                    }
                    None
                }
                StreamEvent::Boundary {
                    kind: StreamBoundaryKind::UserAck,
                    ..
                } => {
                    buf.closed_after_user_ack = true;
                    None
                }
                StreamEvent::Done => {
                    let merged = std::mem::take(&mut buf.text);
                    if merged.trim().is_empty() {
                        None
                    } else {
                        Some(merged)
                    }
                }
                StreamEvent::ToolUse { .. } | StreamEvent::ToolResult { .. } => None,
            }
        };

        let Some(merged) = maybe_message else {
            return;
        };

        let outbound_text = format_scheduled_message(&merged, &thread_id);
        let dispatcher = dispatcher.clone();
        let router = router.clone();
        let request = OutboundMessage {
            channel: channel.clone(),
            account_id: account_id.clone(),
            chat_id: chat_id.clone(),
            delivery_target_type: delivery_target_type.clone(),
            delivery_target_id: delivery_target_id.clone(),
            text: outbound_text,
            reply_to: None,
            thread_id: delivery_thread_id.clone(),
        };
        let channel_name = channel.clone();
        let account_name = account_id.clone();
        let chat_id_value = chat_id.clone();
        let thread_key_value = thread_id.clone();
        let delivery_thread_id_value = delivery_thread_id.clone();
        let thread_log_id_value = thread_log_id.clone();

        tokio::spawn(async move {
            match dispatcher.send_message(request).await {
                Ok(SendMessageResult { message_ids }) => {
                    if message_ids.is_empty() {
                        return;
                    }
                    let mut router_guard = router.lock().await;
                    for message_id in message_ids {
                        router_guard
                            .record_outbound_message_with_thread_log(
                                &thread_key_value,
                                &channel_name,
                                &account_name,
                                &chat_id_value,
                                delivery_thread_id_value.as_deref(),
                                &message_id,
                                thread_log_id_value.as_deref(),
                            )
                            .await;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to send scheduled heartbeat response");
                }
            }
        });
    })
}

fn append_inline_assistant_separator(buffer: &mut String) {
    if buffer.trim().is_empty() || buffer.ends_with("\n\n") {
        return;
    }
    if buffer.ends_with('\n') {
        buffer.push('\n');
    } else {
        buffer.push_str("\n\n");
    }
}

#[derive(Default)]
struct ScheduledStreamState {
    text: String,
    closed_after_user_ack: bool,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
