use super::*;
use async_trait::async_trait;
use garyx_bridge::{BridgeError, ProviderRuntime};
use garyx_channels::{ChannelDispatcher, ChannelInfo, OutboundMessage};
use garyx_models::config::{CronAction, CronConfig, CronJobConfig, CronSchedule};
use garyx_models::provider::{
    ProviderRunOptions, ProviderRunResult, ProviderType, StreamBoundaryKind, StreamEvent,
};
use garyx_models::thread_logs::{NoopThreadLogSink, ThreadLogChunk, ThreadLogEvent, ThreadLogSink};
use garyx_router::ThreadStore;
use tempfile::TempDir;

/// Test [`AutomationDispatchPort`]: the readiness flag is fixed, and any
/// front-door dispatch degrades to `StateUnavailable` — mirroring an engine
/// whose gateway state is gone (or, with `ready: false`, still starting).
struct TestDispatchPort {
    ready: bool,
}

#[async_trait]
impl AutomationDispatchPort for TestDispatchPort {
    fn provider_runtime_ready(&self) -> bool {
        self.ready
    }

    async fn invalidate_gateway_sync_caches(&self) {}

    async fn dispatch_internal_message(
        &self,
        _thread_id: &str,
        _run_id: &str,
        _message: &str,
        _extra_metadata: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<garyx_models::provider::AgentDispatchOutcome, AutomationDispatchError> {
        Err(AutomationDispatchError::StateUnavailable)
    }
}

/// Execution env over `store` with no gateway state behind the port: the
/// ready gate is open, but any front-door dispatch reports
/// `StateUnavailable`. Mirrors the retired "dispatch runtime installed,
/// AppState back-reference missing" fixture semantics.
fn stateless_exec_env(store: Arc<dyn ThreadStore>) -> AutomationExecEnv {
    stateless_exec_env_with(store, Arc::new(MultiProviderBridge::new()), true)
}

fn stateless_exec_env_with(
    store: Arc<dyn ThreadStore>,
    bridge: Arc<MultiProviderBridge>,
    ready: bool,
) -> AutomationExecEnv {
    AutomationExecEnv {
        thread_store: store.clone(),
        router: Arc::new(tokio::sync::Mutex::new(MessageRouter::new(
            store,
            garyx_models::config::GaryxConfig::default(),
        ))),
        bridge,
        thread_logs: Arc::new(NoopThreadLogSink),
        custom_agents: Arc::new(crate::custom_agents::CustomAgentStore::new()),
        garyx_db: None,
        port: Arc::new(TestDispatchPort { ready }),
    }
}

/// Execution env for jobs that never touch the thread store (Log actions,
/// schedule math, persistence round-trips).
fn bare_exec_env() -> AutomationExecEnv {
    stateless_exec_env(Arc::new(garyx_router::InMemoryThreadStore::new()))
}

#[derive(Default)]
struct RecordingDispatcher {
    calls: std::sync::Mutex<Vec<OutboundMessage>>,
    message_ids: std::sync::Mutex<Vec<String>>,
}

#[derive(Default)]
struct RecordingThreadLogSink {
    events: std::sync::Mutex<Vec<ThreadLogEvent>>,
}

impl RecordingThreadLogSink {
    fn events(&self) -> Vec<ThreadLogEvent> {
        self.events
            .lock()
            .expect("recording thread log lock poisoned")
            .clone()
    }
}

#[async_trait]
impl ThreadLogSink for RecordingThreadLogSink {
    async fn record_event(&self, event: ThreadLogEvent) {
        self.events
            .lock()
            .expect("recording thread log lock poisoned")
            .push(event);
    }

    async fn read_chunk(
        &self,
        thread_id: &str,
        cursor: Option<u64>,
    ) -> Result<ThreadLogChunk, String> {
        Ok(ThreadLogChunk {
            thread_id: thread_id.to_owned(),
            path: String::new(),
            text: String::new(),
            cursor: cursor.unwrap_or_default(),
            reset: cursor.is_none(),
        })
    }

    async fn delete_thread(&self, _thread_id: &str) -> Result<(), String> {
        Ok(())
    }
}

impl RecordingDispatcher {
    fn with_message_ids(ids: Vec<String>) -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            message_ids: std::sync::Mutex::new(ids),
        }
    }

    fn calls(&self) -> Vec<OutboundMessage> {
        self.calls
            .lock()
            .expect("recording dispatcher lock poisoned")
            .clone()
    }
}

#[async_trait]
impl ChannelDispatcher for RecordingDispatcher {
    async fn send_message(
        &self,
        request: OutboundMessage,
    ) -> Result<SendMessageResult, garyx_channels::ChannelError> {
        self.calls
            .lock()
            .expect("recording dispatcher lock poisoned")
            .push(request);
        Ok(SendMessageResult {
            message_ids: self
                .message_ids
                .lock()
                .expect("recording dispatcher message_ids lock poisoned")
                .clone(),
        })
    }

    fn available_channels(&self) -> Vec<ChannelInfo> {
        vec![ChannelInfo {
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            is_running: true,
        }]
    }
}

struct SuccessfulAutomationProvider;
struct CountingAutomationProvider {
    calls: std::sync::atomic::AtomicUsize,
    delay_ms: u64,
    provider_type: ProviderType,
}

/// Records every `run_streaming` invocation's thread, message, and metadata
/// so tests can assert what actually reached the provider through the
/// internal-inbound front door.
#[derive(Default)]
struct MetadataRecordingProvider {
    calls: std::sync::Mutex<Vec<(String, String, HashMap<String, serde_json::Value>)>>,
}

#[async_trait]
impl ProviderRuntime for MetadataRecordingProvider {
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
        self.calls.lock().unwrap().push((
            options.thread_id.clone(),
            options.message.clone(),
            options.metadata.clone(),
        ));
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "metadata-recording-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: "ok".to_owned(),
            session_messages: vec![],
            sdk_session_id: None,
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 0,
            output_tokens: 0,
            cost: 0.0,
            duration_ms: 0,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(session_key.to_owned())
    }
}

/// Wire a cron service and a pre-registered bridge into a production-shaped
/// `AppState` so AgentTurn jobs that resolve to a real thread can dispatch
/// through the internal-inbound front door. Returns the state plus the
/// production execution env built from it (same thread store and router).
async fn wire_front_door_state(
    svc: &Arc<CronService>,
    bridge: Arc<MultiProviderBridge>,
) -> (Arc<crate::server::AppState>, AutomationExecEnv) {
    wire_front_door_state_with_agents(
        svc,
        bridge,
        Arc::new(crate::custom_agents::CustomAgentStore::new()),
    )
    .await
}

async fn wire_front_door_state_with_agents(
    svc: &Arc<CronService>,
    bridge: Arc<MultiProviderBridge>,
    custom_agents: Arc<crate::custom_agents::CustomAgentStore>,
) -> (Arc<crate::server::AppState>, AutomationExecEnv) {
    let state = crate::composition::app_bootstrap::AppStateBuilder::new(
        garyx_models::config::GaryxConfig::default(),
    )
    .with_bridge(bridge.clone())
    .with_cron_service(svc.clone())
    .with_custom_agent_store(custom_agents.clone())
    .build();
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    bridge.set_event_tx(state.ops.events.sender()).await;
    let env = crate::composition::automation_wiring::automation_exec_env(&state);
    (state, env)
}

#[async_trait]
impl ProviderRuntime for SuccessfulAutomationProvider {
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
            text: format!("ok: {}", options.message),
        });
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "cron-success-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: format!("ok: {}", options.message),
            session_messages: vec![],
            sdk_session_id: None,
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 0,
            output_tokens: 0,
            cost: 0.0,
            duration_ms: 0,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(session_key.to_owned())
    }
}

impl CountingAutomationProvider {
    fn new(delay_ms: u64) -> Self {
        Self::new_with_type(delay_ms, ProviderType::ClaudeCode)
    }

    fn new_with_type(delay_ms: u64, provider_type: ProviderType) -> Self {
        Self {
            calls: std::sync::atomic::AtomicUsize::new(0),
            delay_ms,
            provider_type,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl ProviderRuntime for CountingAutomationProvider {
    fn provider_type(&self) -> ProviderType {
        self.provider_type.clone()
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
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(tokio::time::Duration::from_millis(self.delay_ms)).await;
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "cron-counting-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: String::new(),
            session_messages: vec![],
            sdk_session_id: None,
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 0,
            output_tokens: 0,
            cost: 0.0,
            duration_ms: self.delay_ms as i64,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(session_key.to_owned())
    }
}

fn make_job_config(id: &str, interval_secs: u64) -> CronJobConfig {
    CronJobConfig {
        id: id.to_owned(),
        kind: Default::default(),
        label: None,
        schedule: CronSchedule::Interval { interval_secs },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    }
}

#[tokio::test]
async fn test_add_list_delete() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());

    // Add
    let job = svc.add(make_job_config("j1", 60)).await.unwrap();
    assert_eq!(job.id, "j1");
    assert_eq!(job.last_status, JobRunStatus::NeverRun);

    // List
    let jobs = svc.list().await;
    assert_eq!(jobs.len(), 1);

    // Delete
    assert!(svc.delete("j1").await.unwrap());
    assert!(!svc.delete("j1").await.unwrap());
    assert!(svc.list().await.is_empty());
}

#[tokio::test]
async fn test_add_creates_storage_dirs_when_missing() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("fresh-cron-root");
    let svc = CronService::new(data_dir.clone());

    let job = svc.add(make_job_config("fresh", 60)).await.unwrap();

    assert_eq!(job.id, "fresh");
    assert!(jobs_dir(&data_dir).join("fresh.json").exists());
}

#[tokio::test]
async fn test_add_accepts_zero_interval_schedule() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());

    let job = svc.add(make_job_config("zero", 0)).await.unwrap();
    assert_eq!(job.schedule, CronSchedule::Interval { interval_secs: 0 });
    assert_eq!(job.next_run, job.created_at);
}

#[tokio::test]
async fn test_tick_does_not_claim_provider_job_before_runtime_ready() {
    let tmp = TempDir::new().unwrap();
    let cron = Arc::new(CronService::new(tmp.path().to_path_buf()));
    let mut cfg = make_job_config("startup-agent", 0);
    cfg.action = CronAction::AgentTurn;
    cfg.target = Some("thread::startup-agent".to_owned());
    cfg.message = Some("ping".to_owned());
    cron.add(cfg).await.unwrap();

    let _state = crate::composition::app_bootstrap::AppStateBuilder::new(
        garyx_models::config::GaryxConfig::default(),
    )
    .with_cron_service(cron.clone())
    .with_provider_runtime_ready(false)
    .build();

    CronService::tick(
        &cron.jobs,
        &cron.runs,
        &cron.active_agent_runs,
        tmp.path(),
        &crate::composition::automation_wiring::automation_exec_env(&_state),
    )
    .await;

    assert!(
        cron.list_runs(10, 0).await.is_empty(),
        "provider-backed cron jobs should not record a failed run during startup"
    );
    assert_eq!(
        cron.get("startup-agent").await.unwrap().last_status,
        JobRunStatus::NeverRun
    );
}

#[tokio::test]
async fn test_tick_does_not_claim_provider_job_when_state_is_unavailable() {
    let tmp = TempDir::new().unwrap();
    let cron = CronService::new(tmp.path().to_path_buf());
    let mut cfg = make_job_config("pre-state-agent", 0);
    cfg.action = CronAction::AgentTurn;
    cfg.target = Some("thread::pre-state-agent".to_owned());
    cfg.message = Some("ping".to_owned());
    cron.add(cfg).await.unwrap();

    CronService::tick(
        &cron.jobs,
        &cron.runs,
        &cron.active_agent_runs,
        tmp.path(),
        &stateless_exec_env_with(
            Arc::new(garyx_router::InMemoryThreadStore::new()),
            Arc::new(MultiProviderBridge::new()),
            false,
        ),
    )
    .await;

    assert!(
        cron.list_runs(10, 0).await.is_empty(),
        "a scheduler whose gateway state is unavailable must not record a startup failure"
    );
    assert_eq!(
        cron.get("pre-state-agent").await.unwrap().last_status,
        JobRunStatus::NeverRun
    );
}

#[tokio::test]
async fn test_add_rejects_too_large_interval_schedule() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());

    let error = svc
        .add(make_job_config("huge", (i64::MAX as u64) + 1))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("exceeds max interval_secs"));
}

#[tokio::test]
async fn test_add_rejects_interval_that_would_overflow_timeline() {
    // ~3e9 hours: fits in i64 and passed the old (i64::MAX) cap, but adding it
    // to `now` overflows chrono's DateTime and used to panic inside
    // compute_next_run -- crashing the create request and, on the update path,
    // eventually the whole scheduler task.
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());

    let error = svc
        .add(make_job_config("overflow", 3_000_000_000 * 3600))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("exceeds max interval_secs"));
}

#[test]
fn from_config_parks_overflow_interval_far_in_future_without_panicking() {
    // Defense in depth: even if an overflow-sized interval reaches
    // compute_next_run (e.g. a legacy persisted job that bypassed validation),
    // it must not panic. The run is parked far in the future instead of killing
    // the scheduler task.
    let job = CronJob::from_config(&make_job_config("overflow", 3_000_000_000 * 3600));
    assert!(job.next_run > Utc::now() + chrono::Duration::days(3650));

    // The interval that also overflows chrono's Duration constructor itself.
    let job = CronJob::from_config(&make_job_config("huge", i64::MAX as u64));
    assert!(job.next_run > Utc::now() + chrono::Duration::days(3650));
}

#[tokio::test]
async fn load_resets_stale_running_status_so_job_fires_again() {
    // A job killed mid-run (Garyx restarts are non-graceful / SIGKILL and never
    // settle the in-flight tick) leaves last_status = Running persisted on disk.
    // On reload it must be reset, or claim_job_for_execution skips it forever
    // (the guard rejects any job whose last_status == Running) and the schedule
    // silently stops firing with no recovery via the UI.
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().to_path_buf();
    ensure_dirs(&data_dir).await.unwrap();

    // Simulate a crash mid-run: a job persisted while claimed as Running.
    let mut job = CronJob::from_config(&make_job_config("wedged", 60));
    job.last_status = JobRunStatus::Running;
    persist_job(&data_dir, &job).await.unwrap();

    // Reload as the gateway does on startup.
    let svc = CronService::new(data_dir);
    svc.load(&CronConfig::default()).await.unwrap();

    let loaded = svc.get("wedged").await.expect("job present after load");
    assert_ne!(
        loaded.last_status,
        JobRunStatus::Running,
        "a stale Running left by a previous run must be reset on load or the job never fires again",
    );
}

#[tokio::test]
async fn test_add_rejects_invalid_once_timestamp() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());

    let error = svc
        .add(CronJobConfig {
            id: "once-invalid".to_owned(),
            kind: Default::default(),
            label: None,
            schedule: CronSchedule::Once {
                at: "not-a-time".to_owned(),
            },
            ui_schedule: None,
            action: CronAction::Log,
            target: None,
            message: None,
            workspace_dir: None,
            agent_id: None,
            thread_id: None,
            delete_after_run: false,
            enabled: true,
            system: false,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("invalid once timestamp"));
}

#[tokio::test]
async fn test_add_trims_once_timestamp_schedule() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let at = " 2026-03-08T16:00:00Z ";

    let job = svc
        .add(CronJobConfig {
            id: "once-trimmed".to_owned(),
            kind: Default::default(),
            label: None,
            schedule: CronSchedule::Once { at: at.to_owned() },
            ui_schedule: None,
            action: CronAction::Log,
            target: None,
            message: None,
            workspace_dir: None,
            agent_id: None,
            thread_id: None,
            delete_after_run: false,
            enabled: true,
            system: false,
        })
        .await
        .unwrap();

    assert_eq!(
        job.next_run,
        "2026-03-08T16:00:00Z".parse::<DateTime<Utc>>().unwrap()
    );
}

#[tokio::test]
async fn test_add_accepts_once_protocol_timestamp() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());

    let job = svc
        .add(CronJobConfig {
            id: "once-protocol".to_owned(),
            kind: Default::default(),
            label: None,
            schedule: CronSchedule::Once {
                at: "ONCE:2026-03-08 16:00".to_owned(),
            },
            ui_schedule: None,
            action: CronAction::Log,
            target: None,
            message: None,
            workspace_dir: None,
            agent_id: None,
            thread_id: None,
            delete_after_run: false,
            enabled: true,
            system: false,
        })
        .await
        .unwrap();

    let expected = crate::cron::parse_once_timestamp("ONCE:2026-03-08 16:00").unwrap();
    assert_eq!(job.next_run, expected);
}

#[tokio::test]
async fn test_persistence_survives_reload() {
    let tmp = TempDir::new().unwrap();

    // First instance: add jobs.
    {
        let svc = CronService::new(tmp.path().to_path_buf());
        let _ = ensure_dirs(tmp.path()).await;
        svc.add(make_job_config("p1", 120)).await.unwrap();
        svc.add(make_job_config("p2", 300)).await.unwrap();
    }

    // Second instance: load from disk.
    {
        let svc = CronService::new(tmp.path().to_path_buf());
        let cfg = garyx_models::config::CronConfig { jobs: Vec::new() };
        svc.load(&cfg).await.unwrap();
        let jobs = svc.list().await;
        assert_eq!(jobs.len(), 2);
    }
}

#[tokio::test]
async fn test_load_updates_automation_fields_from_config() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    svc.add(CronJobConfig {
        id: "auto-merge".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Old Label".to_owned()),
        schedule: CronSchedule::Interval {
            interval_secs: 3600,
        },
        ui_schedule: None,
        action: CronAction::AgentTurn,
        target: None,
        message: Some("old prompt".to_owned()),
        workspace_dir: Some("/tmp/old-workspace".to_owned()),
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .unwrap();

    svc.load(&CronConfig {
        jobs: vec![CronJobConfig {
            id: "auto-merge".to_owned(),
            kind: CronJobKind::AutomationPrompt,
            label: Some("New Label".to_owned()),
            schedule: CronSchedule::Interval { interval_secs: 60 },
            ui_schedule: Some(garyx_models::config::AutomationScheduleView::Interval { hours: 1 }),
            action: CronAction::AgentTurn,
            target: None,
            message: Some("new prompt".to_owned()),
            workspace_dir: Some("/tmp/new-workspace".to_owned()),
            agent_id: None,
            thread_id: Some("thread::manual".to_owned()),
            delete_after_run: false,
            enabled: true,
            system: false,
        }],
    })
    .await
    .unwrap();

    let job = svc.get("auto-merge").await.expect("merged automation job");
    assert_eq!(job.label.as_deref(), Some("New Label"));
    assert_eq!(job.message.as_deref(), Some("new prompt"));
    assert_eq!(job.workspace_dir.as_deref(), Some("/tmp/new-workspace"));
    assert_eq!(job.thread_id.as_deref(), Some("thread::manual"));
    assert_eq!(
        job.ui_schedule,
        Some(garyx_models::config::AutomationScheduleView::Interval { hours: 1 })
    );
}

#[tokio::test]
async fn test_load_skips_invalid_persisted_schedule() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    ensure_dirs(tmp.path()).await.unwrap();
    persist_job(
        tmp.path(),
        &CronJob {
            id: "bad-persisted".to_owned(),
            kind: Default::default(),
            label: None,
            schedule: CronSchedule::Once {
                at: "not-a-time".to_owned(),
            },
            ui_schedule: None,
            action: CronAction::Log,
            target: None,
            message: None,
            workspace_dir: None,
            agent_id: None,
            thread_id: None,
            delete_after_run: false,
            enabled: true,
            next_run: Utc::now(),
            last_status: JobRunStatus::NeverRun,
            run_count: 0,
            created_at: Utc::now(),
            last_run_at: None,
            system: false,
            validation_error: None,
        },
    )
    .await
    .unwrap();

    svc.load(&CronConfig::default()).await.unwrap();
    assert!(svc.list().await.is_empty());
    assert!(!jobs_dir(tmp.path()).join("bad-persisted.json").exists());
}

#[tokio::test]
async fn test_load_skips_invalid_runs_history() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    ensure_dirs(tmp.path()).await.unwrap();
    tokio::fs::write(runs_file(tmp.path()), b"{ not-json")
        .await
        .unwrap();

    svc.load(&CronConfig::default()).await.unwrap();

    assert!(svc.list_runs(10, 0).await.is_empty());
    assert!(!runs_file(tmp.path()).exists());
}

#[tokio::test]
async fn test_load_skips_invalid_config_schedule() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let cfg = CronConfig {
        jobs: vec![
            make_job_config("valid", 60),
            CronJobConfig {
                id: "invalid".to_owned(),
                kind: Default::default(),
                label: None,
                schedule: CronSchedule::Once {
                    at: "not-a-time".to_owned(),
                },
                ui_schedule: None,
                action: CronAction::Log,
                target: None,
                message: None,
                workspace_dir: None,
                agent_id: None,
                thread_id: None,
                delete_after_run: false,
                enabled: true,
                system: false,
            },
        ],
    };

    svc.load(&cfg).await.unwrap();
    let jobs = svc.list().await;
    assert_eq!(jobs.len(), 1);
    assert!(jobs.iter().any(|job| job.id == "valid"));
}

#[tokio::test]
async fn test_run_now() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    svc.add(make_job_config("rn1", 9999)).await.unwrap();
    let record = svc.run_now("rn1", &bare_exec_env()).await.unwrap();
    assert_eq!(record.job_id, "rn1");
    assert_eq!(record.status, JobRunStatus::Success);
    assert!(record.duration_ms.is_some());

    // Run count should be 1.
    let jobs = svc.list().await;
    let j = jobs.iter().find(|j| j.id == "rn1").unwrap();
    assert_eq!(j.run_count, 1);

    // Run history should include the run.
    let runs = svc.list_runs(10, 0).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].job_id, "rn1");
}

#[tokio::test]
async fn test_run_now_advances_interval_schedule() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    svc.add(make_job_config("rn-advance", 60)).await.unwrap();
    let before = svc
        .list()
        .await
        .into_iter()
        .find(|job| job.id == "rn-advance")
        .unwrap()
        .next_run;

    let record = svc.run_now("rn-advance", &bare_exec_env()).await.unwrap();
    assert_eq!(record.status, JobRunStatus::Success);

    let after = svc
        .list()
        .await
        .into_iter()
        .find(|job| job.id == "rn-advance")
        .unwrap();
    assert!(after.next_run > before);
    assert_eq!(after.run_count, 1);
}

#[tokio::test]
async fn test_run_now_disables_once_job_after_success() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    svc.add(CronJobConfig {
        id: "rn-once".to_owned(),
        kind: Default::default(),
        label: None,
        schedule: CronSchedule::Once {
            at: "2030-01-01T00:00:00Z".to_owned(),
        },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .unwrap();

    let record = svc.run_now("rn-once", &bare_exec_env()).await.unwrap();
    assert_eq!(record.status, JobRunStatus::Success);

    let job = svc
        .list()
        .await
        .into_iter()
        .find(|job| job.id == "rn-once")
        .unwrap();
    assert!(!job.enabled);
    assert_eq!(job.run_count, 1);
}

#[tokio::test]
async fn test_tick_failure_does_not_advance_schedule() {
    let tmp = TempDir::new().unwrap();
    let svc = Arc::new(CronService::new(tmp.path().to_path_buf()));
    let _ = ensure_dirs(tmp.path()).await;

    svc.add(CronJobConfig {
        id: "tick-fail".to_owned(),
        kind: Default::default(),
        label: None,
        schedule: CronSchedule::Interval { interval_secs: 0 },
        ui_schedule: None,
        action: CronAction::AgentTurn,
        target: None,
        message: Some("exercise runtime failure".to_owned()),
        workspace_dir: Some("/tmp".to_owned()),
        agent_id: Some("claude".to_owned()),
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .unwrap();

    let before = svc
        .list()
        .await
        .into_iter()
        .find(|job| job.id == "tick-fail")
        .unwrap()
        .next_run;

    CronService::tick(
        &svc.jobs,
        &svc.runs,
        &svc.active_agent_runs,
        tmp.path(),
        &stateless_exec_env(Arc::new(garyx_router::InMemoryThreadStore::new())),
    )
    .await;

    let job = svc
        .list()
        .await
        .into_iter()
        .find(|job| job.id == "tick-fail")
        .unwrap();
    assert_eq!(job.last_status, JobRunStatus::Failed);
    assert_eq!(job.next_run, before);
    assert_eq!(job.run_count, 1);
    assert!(job.last_run_at.is_some());

    let runs = svc.list_runs(10, 0).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, JobRunStatus::Failed);
}

#[tokio::test]
async fn test_run_now_delete_after_run_removes_job_and_file() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    svc.add(CronJobConfig {
        id: "delete-now".to_owned(),
        kind: Default::default(),
        label: None,
        schedule: CronSchedule::Interval {
            interval_secs: 9999,
        },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: None,
        delete_after_run: true,
        enabled: true,
        system: false,
    })
    .await
    .unwrap();

    let record = svc.run_now("delete-now", &bare_exec_env()).await.unwrap();
    assert_eq!(record.status, JobRunStatus::Success);

    let jobs = svc.list().await;
    assert!(jobs.iter().all(|j| j.id != "delete-now"));
    assert!(!jobs_dir(tmp.path()).join("delete-now.json").exists());
}

#[tokio::test]
async fn test_build_scheduled_response_callback_sends_final_message() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let callback = build_scheduled_response_callback(
        dispatcher.clone(),
        Arc::new(NoopThreadLogSink),
        ScheduledResponseContext {
            thread_id: "cron::daily".to_owned(),
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            chat_id: "42".to_owned(),
            delivery_target_type: "chat_id".to_owned(),
            delivery_target_id: "42".to_owned(),
            delivery_thread_id: Some("100".to_owned()),
            thread_log_id: Some("thread::cron-log".to_owned()),
        },
    );

    callback(StreamEvent::Delta {
        text: "hello ".to_owned(),
    });
    callback(StreamEvent::Delta {
        text: "world".to_owned(),
    });
    callback(StreamEvent::Done);

    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].channel, "telegram");
    assert_eq!(calls[0].account_id, "main");
    assert_eq!(calls[0].chat_id, "42");
    assert_eq!(calls[0].thread_id.as_deref(), Some("100"));
    assert_eq!(calls[0].text_content(), Some("#cron::daily\nhello world"));
}

#[tokio::test]
async fn test_build_scheduled_response_callback_records_delivery_log_per_message_id() {
    let dispatcher = Arc::new(RecordingDispatcher::with_message_ids(vec![
        "cron_msg_1".to_owned(),
        "cron_msg_2".to_owned(),
    ]));
    let thread_logs = Arc::new(RecordingThreadLogSink::default());
    let callback = build_scheduled_response_callback(
        dispatcher,
        thread_logs.clone(),
        ScheduledResponseContext {
            thread_id: "cron::daily".to_owned(),
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            chat_id: "42".to_owned(),
            delivery_target_type: "chat_id".to_owned(),
            delivery_target_id: "42".to_owned(),
            delivery_thread_id: Some("42_t100".to_owned()),
            thread_log_id: Some("thread::cron-log".to_owned()),
        },
    );

    callback(StreamEvent::Delta {
        text: "ping".to_owned(),
    });
    callback(StreamEvent::Done);
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    let events = thread_logs.events();
    assert_eq!(events.len(), 2);
    for (event, message_id) in events.iter().zip(["cron_msg_1", "cron_msg_2"]) {
        assert_eq!(event.thread_id, "thread::cron-log");
        assert_eq!(event.stage, "delivery");
        assert_eq!(event.message, "outbound message delivered");
        assert_eq!(
            event.fields.get("channel"),
            Some(&serde_json::json!("telegram"))
        );
        assert_eq!(
            event.fields.get("account_id"),
            Some(&serde_json::json!("main"))
        );
        assert_eq!(event.fields.get("chat_id"), Some(&serde_json::json!("42")));
        assert_eq!(
            event.fields.get("message_id"),
            Some(&serde_json::json!(message_id))
        );
        assert_eq!(
            event.fields.get("thread_id"),
            Some(&serde_json::json!("thread::cron-log"))
        );
    }
}

#[tokio::test]
async fn test_build_scheduled_response_callback_preserves_assistant_segments() {
    let dispatcher = Arc::new(RecordingDispatcher::with_message_ids(vec![
        "cron_msg_1".to_owned(),
    ]));
    let callback = build_scheduled_response_callback(
        dispatcher.clone(),
        Arc::new(NoopThreadLogSink),
        ScheduledResponseContext {
            thread_id: "cron::daily".to_owned(),
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            chat_id: "42".to_owned(),
            delivery_target_type: "chat_id".to_owned(),
            delivery_target_id: "42".to_owned(),
            delivery_thread_id: None,
            thread_log_id: None,
        },
    );

    callback(StreamEvent::Delta {
        text: "first".to_owned(),
    });
    callback(StreamEvent::Boundary {
        kind: StreamBoundaryKind::AssistantSegment,
        pending_input_id: None,
    });
    callback(StreamEvent::Delta {
        text: "second".to_owned(),
    });
    callback(StreamEvent::Done);
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].text_content(),
        Some("#cron::daily\nfirst\n\nsecond")
    );
}

#[tokio::test]
async fn test_build_scheduled_response_callback_stops_after_user_ack_boundary() {
    let dispatcher = Arc::new(RecordingDispatcher::with_message_ids(vec![
        "cron_msg_1".to_owned(),
    ]));
    let callback = build_scheduled_response_callback(
        dispatcher.clone(),
        Arc::new(NoopThreadLogSink),
        ScheduledResponseContext {
            thread_id: "cron::daily".to_owned(),
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            chat_id: "42".to_owned(),
            delivery_target_type: "chat_id".to_owned(),
            delivery_target_id: "42".to_owned(),
            delivery_thread_id: None,
            thread_log_id: None,
        },
    );

    callback(StreamEvent::Delta {
        text: "first".to_owned(),
    });
    callback(StreamEvent::Boundary {
        kind: StreamBoundaryKind::UserAck,
        pending_input_id: None,
    });
    callback(StreamEvent::Delta {
        text: "second".to_owned(),
    });
    callback(StreamEvent::Done);
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].text_content(), Some("#cron::daily\nfirst"));
}

#[tokio::test]
async fn test_dispatch_agent_turn_recovers_thread_target_delivery_from_store() {
    let store: Arc<dyn ThreadStore> = Arc::new(garyx_router::InMemoryThreadStore::new());
    let thread_id = "bot1::main::u1";
    store
        .set(
            thread_id,
            serde_json::json!({
                "lastChannel": "telegram",
                "lastTo": "42",
                "lastAccountId": "main",
                "lastUpdatedAt": "2026-03-01T12:00:00Z",
            }),
        )
        .await
        .unwrap();
    let env = stateless_exec_env(store);
    let job = CronJob {
        id: "recover-delivery".to_owned(),
        kind: Default::default(),
        label: None,
        schedule: CronSchedule::Interval { interval_secs: 60 },
        ui_schedule: None,
        action: CronAction::Log,
        target: Some(format!("thread:{thread_id}")),
        message: Some("ping".to_owned()),
        workspace_dir: None,
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        next_run: Utc::now(),
        last_status: JobRunStatus::NeverRun,
        run_count: 0,
        created_at: Utc::now(),
        last_run_at: None,
        system: false,
        validation_error: None,
    };

    let active_agent_runs = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    // A thread-like target whose record exists routes through the
    // internal-inbound front door; this fixture's port has no gateway state
    // behind it, so the front-door branch is the one that must fail.
    let err = CronService::dispatch_agent_turn(&job, "run-1", "ping", &active_agent_runs, &env)
        .await
        .expect_err("front door requires an available gateway state");
    assert!(err.contains("gateway app state is unavailable"));
}

#[tokio::test]
async fn test_successful_automation_run_persists_thread_id() {
    let tmp = TempDir::new().unwrap();
    let svc = Arc::new(CronService::new(tmp.path().to_path_buf()));
    let _ = ensure_dirs(tmp.path()).await;

    svc.add(CronJobConfig {
        id: "automation-persist".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Automation Persist".to_owned()),
        schedule: CronSchedule::Interval { interval_secs: 60 },
        ui_schedule: None,
        action: CronAction::AgentTurn,
        target: None,
        message: Some("ping".to_owned()),
        workspace_dir: Some("/tmp/automation-persist".to_owned()),
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .unwrap();

    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("automation-success", Arc::new(SuccessfulAutomationProvider))
        .await;
    bridge.set_default_provider_key("automation-success").await;
    let (_state, env) = wire_front_door_state(&svc, bridge).await;

    let run = svc.run_now("automation-persist", &env).await.unwrap();
    assert_eq!(run.status, JobRunStatus::Success);
    assert!(run.thread_id.is_some());

    let reloaded = CronService::new(tmp.path().to_path_buf());
    reloaded.load(&CronConfig::default()).await.unwrap();
    let reloaded_job = reloaded
        .get("automation-persist")
        .await
        .expect("reloaded automation job");

    assert!(reloaded_job.thread_id.is_none());
}

#[tokio::test]
async fn generated_automation_disabled_at_run_time_records_visible_failure() {
    let tmp = TempDir::new().unwrap();
    let svc = Arc::new(CronService::new(tmp.path().to_path_buf()));
    svc.add(CronJobConfig {
        id: "automation-disabled-at-run".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Disabled at run".to_owned()),
        schedule: CronSchedule::Interval { interval_secs: 60 },
        ui_schedule: None,
        action: CronAction::AgentTurn,
        target: None,
        message: Some("must fail visibly".to_owned()),
        workspace_dir: Some("/tmp/automation-disabled-at-run".to_owned()),
        agent_id: Some("claude".to_owned()),
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .unwrap();

    let custom_agents = Arc::new(crate::custom_agents::CustomAgentStore::new());
    custom_agents
        .set_enabled("claude", false)
        .await
        .expect("disable bound generated agent after configuration");
    let provider = Arc::new(CountingAutomationProvider::new(0));
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("disabled-at-run-provider", provider.clone())
        .await;
    bridge
        .set_default_provider_key("disabled-at-run-provider")
        .await;
    let (state, env) = wire_front_door_state_with_agents(&svc, bridge, custom_agents).await;

    let run = svc.run_now("automation-disabled-at-run", &env).await.unwrap();
    assert_eq!(run.status, JobRunStatus::Failed);
    assert!(
        run.error
            .as_deref()
            .is_some_and(|error| error.contains("agent is disabled: claude"))
    );
    assert_eq!(provider.calls(), 0);
    assert!(
        state
            .threads
            .thread_store
            .list_keys(Some("thread::"))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn test_bound_automation_run_reuses_existing_thread() {
    let tmp = TempDir::new().unwrap();
    let svc = Arc::new(CronService::new(tmp.path().to_path_buf()));
    let _ = ensure_dirs(tmp.path()).await;

    let target_thread_id = "thread::bound-automation";
    svc.add(CronJobConfig {
        id: "automation-bound".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Automation Bound".to_owned()),
        schedule: CronSchedule::Interval { interval_secs: 60 },
        ui_schedule: None,
        action: CronAction::AgentTurn,
        target: None,
        message: Some("ping".to_owned()),
        workspace_dir: None,
        agent_id: None,
        thread_id: Some(target_thread_id.to_owned()),
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .unwrap();

    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("automation-success", Arc::new(SuccessfulAutomationProvider))
        .await;
    bridge.set_default_provider_key("automation-success").await;
    let custom_agents = Arc::new(crate::custom_agents::CustomAgentStore::new());
    custom_agents
        .set_enabled("claude", false)
        .await
        .expect("disable existing target binding");
    let (state, env) = wire_front_door_state_with_agents(&svc, bridge, custom_agents).await;
    let store = state.threads.thread_store.clone();
    store
        .set(
            target_thread_id,
            serde_json::json!({
                "thread_id": target_thread_id,
                "agent_id": "claude",
                "provider_type": "claude_code",
                "workspace_dir": "/tmp/bound-automation",
                "metadata": {
                    "agent_id": "claude"
                }
            }),
        )
        .await
        .unwrap();

    let run = svc.run_now("automation-bound", &env).await.unwrap();
    assert_eq!(run.status, JobRunStatus::Success);
    assert_eq!(run.thread_id.as_deref(), Some(target_thread_id));

    let reloaded = CronService::new(tmp.path().to_path_buf());
    reloaded.load(&CronConfig::default()).await.unwrap();
    let reloaded_job = reloaded
        .get("automation-bound")
        .await
        .expect("reloaded automation job");

    assert_eq!(reloaded_job.thread_id.as_deref(), Some(target_thread_id));
    assert!(store.get(target_thread_id).await.unwrap().is_some());
}

#[tokio::test]
async fn target_automation_runtime_uses_live_codex_binding_not_legacy_claude_job_cache() {
    let tmp = TempDir::new().unwrap();
    let svc = Arc::new(CronService::new(tmp.path().to_path_buf()));
    let target_thread_id = "thread::target-live-codex";
    svc.add(CronJobConfig {
        id: "automation-target-live-codex".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Live target binding".to_owned()),
        schedule: CronSchedule::Interval { interval_secs: 60 },
        ui_schedule: None,
        action: CronAction::AgentTurn,
        target: None,
        message: Some("use the target binding".to_owned()),
        workspace_dir: None,
        // Historical target jobs could persist a stale explicit cache. The
        // target thread below is canonically Codex and must win at runtime.
        agent_id: Some("claude".to_owned()),
        thread_id: Some(target_thread_id.to_owned()),
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .unwrap();

    let bridge = Arc::new(MultiProviderBridge::new());
    let claude = Arc::new(CountingAutomationProvider::new_with_type(
        0,
        ProviderType::ClaudeCode,
    ));
    let codex = Arc::new(CountingAutomationProvider::new_with_type(
        0,
        ProviderType::CodexAppServer,
    ));
    bridge
        .register_provider("legacy-claude-provider", claude.clone())
        .await;
    bridge
        .register_provider("live-codex-provider", codex.clone())
        .await;
    bridge
        .set_default_provider_key("legacy-claude-provider")
        .await;
    let (state, env) = wire_front_door_state(&svc, bridge).await;
    state
        .threads
        .thread_store
        .set(
            target_thread_id,
            serde_json::json!({
                "thread_id": target_thread_id,
                "agent_id": "codex",
                "provider_type": "codex_app_server",
                "workspace_dir": "/tmp/target-live-codex",
                "metadata": {
                    "agent_id": "codex",
                    "requested_provider_type": "codex_app_server"
                }
            }),
        )
        .await
        .unwrap();

    let run = svc.run_now("automation-target-live-codex", &env).await.unwrap();
    assert_eq!(run.status, JobRunStatus::Success);
    assert_eq!(run.thread_id.as_deref(), Some(target_thread_id));
    assert_eq!(codex.calls(), 1, "the live target binding selects Codex");
    assert_eq!(
        claude.calls(),
        0,
        "the stale job-level Claude cache must not influence dispatch"
    );
}

#[tokio::test]
async fn test_bound_automation_dispatches_through_internal_inbound_front_door() {
    let tmp = TempDir::new().unwrap();
    let svc = Arc::new(CronService::new(tmp.path().to_path_buf()));
    let _ = ensure_dirs(tmp.path()).await;

    let target_thread_id = "thread::front-door-automation";
    svc.add(CronJobConfig {
        id: "automation-front-door".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Automation Front Door".to_owned()),
        schedule: CronSchedule::Interval { interval_secs: 60 },
        ui_schedule: None,
        action: CronAction::AgentTurn,
        target: None,
        message: Some("scheduled prompt".to_owned()),
        workspace_dir: None,
        agent_id: None,
        thread_id: Some(target_thread_id.to_owned()),
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .unwrap();

    let bridge = Arc::new(MultiProviderBridge::new());
    let provider = Arc::new(MetadataRecordingProvider::default());
    bridge
        .register_provider("front-door-recorder", provider.clone())
        .await;
    bridge.set_default_provider_key("front-door-recorder").await;
    let (state, env) = wire_front_door_state(&svc, bridge).await;
    state
        .threads
        .thread_store
        .set(
            target_thread_id,
            serde_json::json!({
                "thread_id": target_thread_id,
                "workspace_dir": "/tmp/front-door-automation",
                "metadata": { "agent_id": "claude" }
            }),
        )
        .await
        .unwrap();

    let run = svc.run_now("automation-front-door", &env).await.unwrap();
    assert_eq!(run.status, JobRunStatus::Success);

    // The scheduled prompt reached the provider as an ordinary inbound
    // message carrying the internal-dispatch and automation markers.
    let calls = provider.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    let (thread_id, message, metadata) = &calls[0];
    assert_eq!(thread_id, target_thread_id);
    assert_eq!(message, "scheduled prompt");
    assert_eq!(
        metadata.get("internal_dispatch"),
        Some(&serde_json::Value::Bool(true))
    );
    assert_eq!(
        metadata.get("source"),
        Some(&serde_json::json!("automation"))
    );
    assert_eq!(
        metadata.get("automation_id"),
        Some(&serde_json::json!("automation-front-door"))
    );

    // And it was committed to the thread transcript as a user turn — the
    // front door behaves like a person sending a message, not a bare run.
    let latest_user = state
        .threads
        .history
        .latest_message_text_for_role(target_thread_id, "user")
        .await
        .expect("history readable")
        .expect("user turn recorded");
    assert_eq!(latest_user, "scheduled prompt");
}

#[tokio::test]
async fn test_bound_automation_missing_target_thread_fails_without_cleanup() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    let missing_thread_id = "thread::missing-bound-automation";
    svc.add(CronJobConfig {
        id: "automation-missing-bound".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Automation Missing Bound".to_owned()),
        schedule: CronSchedule::Interval { interval_secs: 60 },
        ui_schedule: None,
        action: CronAction::AgentTurn,
        target: None,
        message: Some("ping".to_owned()),
        workspace_dir: None,
        agent_id: None,
        thread_id: Some(missing_thread_id.to_owned()),
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .unwrap();

    let store: Arc<dyn ThreadStore> = Arc::new(garyx_router::InMemoryThreadStore::new());
    let env = stateless_exec_env(store.clone());

    let run = svc.run_now("automation-missing-bound", &env).await.unwrap();

    assert_eq!(run.status, JobRunStatus::Failed);
    assert_eq!(run.thread_id.as_deref(), Some(missing_thread_id));
    assert!(
        run.error
            .as_deref()
            .unwrap_or_default()
            .contains("cron target thread not found")
    );
    let job = svc
        .get("automation-missing-bound")
        .await
        .expect("automation job after failed run");
    assert_eq!(job.thread_id.as_deref(), Some(missing_thread_id));
    assert!(store.get(missing_thread_id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_failed_automation_run_now_cleans_up_failed_thread() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    let store: Arc<dyn ThreadStore> = Arc::new(garyx_router::InMemoryThreadStore::new());

    svc.add(CronJobConfig {
        id: "automation-keep-thread".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Automation Keep Thread".to_owned()),
        schedule: CronSchedule::Interval { interval_secs: 60 },
        ui_schedule: None,
        action: CronAction::AgentTurn,
        target: None,
        message: Some("ping".to_owned()),
        workspace_dir: Some("/tmp/automation-existing".to_owned()),
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .unwrap();

    let env = stateless_exec_env(store.clone());

    let run = svc.run_now("automation-keep-thread", &env).await.unwrap();
    assert_eq!(run.status, JobRunStatus::Failed);
    let failed_thread_id = run.thread_id.clone().expect("failed thread id");

    let job = svc
        .get("automation-keep-thread")
        .await
        .expect("automation job after failed run");
    assert!(job.thread_id.is_none());
    assert!(store.get(&failed_thread_id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_run_now_missing_job() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    assert!(svc.run_now("nonexistent", &bare_exec_env()).await.is_none());
}

#[tokio::test]
async fn test_run_now_disabled_job_is_skipped() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    svc.add(CronJobConfig {
        id: "disabled-now".to_owned(),
        kind: Default::default(),
        label: None,
        schedule: CronSchedule::Interval {
            interval_secs: 9999,
        },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: false,
        system: false,
    })
    .await
    .unwrap();

    assert!(svc.run_now("disabled-now", &bare_exec_env()).await.is_none());
    assert!(svc.list_runs(10, 0).await.is_empty());
}

#[tokio::test]
async fn test_update_job_keeps_runtime_state() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    svc.add(make_job_config("upd1", 60)).await.unwrap();
    svc.run_now("upd1", &bare_exec_env()).await.unwrap();

    let updated = svc
        .update(
            "upd1",
            CronJobConfig {
                id: "upd1".to_owned(),
                kind: Default::default(),
                label: None,
                schedule: CronSchedule::Interval { interval_secs: 120 },
                ui_schedule: None,
                action: CronAction::SystemEvent,
                target: Some("last".to_owned()),
                message: Some("ping".to_owned()),
                workspace_dir: None,
                agent_id: None,
                thread_id: None,
                delete_after_run: true,
                enabled: false,
                system: false,
            },
        )
        .await
        .unwrap()
        .expect("job should exist");

    assert_eq!(updated.run_count, 1);
    assert_eq!(
        updated.schedule,
        CronSchedule::Interval { interval_secs: 120 }
    );
    assert_eq!(updated.action, CronAction::SystemEvent);
    assert_eq!(updated.target.as_deref(), Some("last"));
    assert_eq!(updated.message.as_deref(), Some("ping"));
    assert!(updated.delete_after_run);
    assert!(!updated.enabled);
}

#[tokio::test]
async fn test_job_is_due() {
    let mut job = CronJob::from_config(&make_job_config("due", 0));
    // With interval_secs=0, next_run is essentially now.
    // Force it to the past.
    job.next_run = Utc::now() - chrono::Duration::seconds(1);
    assert!(job.is_due());

    job.enabled = false;
    assert!(!job.is_due());
}

#[tokio::test]
async fn test_once_schedule_disables_after_fire() {
    let cfg = CronJobConfig {
        id: "once1".to_owned(),
        kind: Default::default(),
        label: None,
        schedule: CronSchedule::Once {
            at: Utc::now().to_rfc3339(),
        },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    };
    let mut job = CronJob::from_config(&cfg);
    job.advance();
    assert!(!job.enabled, "one-shot job should disable after firing");
}

#[tokio::test]
async fn test_once_schedule_past_time_is_not_due() {
    let cfg = CronJobConfig {
        id: "once-past".to_owned(),
        kind: Default::default(),
        label: None,
        schedule: CronSchedule::Once {
            at: (Utc::now() - chrono::Duration::minutes(5)).to_rfc3339(),
        },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    };
    let job = CronJob::from_config(&cfg);
    assert!(
        !job.is_due(),
        "past one-shot jobs should not auto-fire on scheduler tick"
    );
}

#[tokio::test]
async fn test_cron_expression_next_run() {
    let cfg = CronJobConfig {
        id: "cron1".to_owned(),
        kind: Default::default(),
        label: None,
        schedule: CronSchedule::Cron {
            // Every minute, second 0
            expr: "0 * * * * *".to_owned(),
            timezone: None,
        },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    };

    let job = CronJob::from_config(&cfg);
    assert!(job.next_run > Utc::now());
}

#[tokio::test]
async fn test_cron_expression_respects_timezone() {
    use chrono::TimeZone;

    let after = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let expr = "0 0 9 * * *".to_owned();

    let bare_schedule = CronSchedule::Cron {
        expr: expr.clone(),
        timezone: None,
    };
    let shanghai_schedule = CronSchedule::Cron {
        expr,
        timezone: Some("Asia/Shanghai".to_owned()),
    };

    let next_bare = CronJob::compute_next_run(&bare_schedule, after);
    let next_shanghai = CronJob::compute_next_run(&shanghai_schedule, after);

    // A bare cron expression (no timezone) is interpreted in the gateway
    // machine's local timezone: the next run lands at 09:00 local wall-clock
    // time regardless of where the test machine is.
    let next_bare_local = next_bare.with_timezone(&Local);
    assert_eq!(
        next_bare_local.format("%H:%M:%S").to_string(),
        "09:00:00",
        "bare cron must fire at 09:00 machine-local time, got {next_bare_local}"
    );
    assert!(next_bare > after);

    // An explicit timezone always wins over the machine's local timezone.
    assert_eq!(
        next_shanghai,
        Utc.with_ymd_and_hms(2026, 1, 1, 1, 0, 0).unwrap()
    );
}

#[tokio::test]
async fn test_cron_dst_fall_back_fires_at_first_occurrence_instead_of_skipping_the_day() {
    use chrono::TimeZone;

    // America/New_York falls back on 2026-11-01: 01:30 local occurs twice
    // (05:30Z as EDT, 06:30Z as EST). The cron crate's own timezone iterator
    // drops the ambiguous wall-clock time and skips the whole day; the
    // schedule must instead fire at the first occurrence.
    let schedule = CronSchedule::Cron {
        expr: "0 30 1 * * *".to_owned(),
        timezone: Some("America/New_York".to_owned()),
    };

    let after = Utc.with_ymd_and_hms(2026, 11, 1, 4, 55, 0).unwrap(); // 00:55 EDT
    let next = CronJob::compute_next_run(&schedule, after);
    assert_eq!(next, Utc.with_ymd_and_hms(2026, 11, 1, 5, 30, 0).unwrap());

    // Re-arming right after that firing lands on the next day: the 01:30
    // wall-clock event already fired once on the fall-back day.
    let rearmed = CronJob::compute_next_run(&schedule, next);
    assert_eq!(
        rearmed,
        Utc.with_ymd_and_hms(2026, 11, 2, 6, 30, 0).unwrap()
    );
}

#[tokio::test]
async fn test_cron_dst_fall_back_never_returns_a_past_instant() {
    use chrono::TimeZone;

    // If `after` already sits inside the *second* pass of the repeated
    // 01:00-02:00 hour (EST side), the ambiguous 01:30 candidate's earlier
    // instant (05:30Z) lies in the past. Returning it would arm next_run in
    // the past and storm-fire every tick; the schedule must pick the
    // still-future second occurrence instead.
    let schedule = CronSchedule::Cron {
        expr: "0 30 1 * * *".to_owned(),
        timezone: Some("America/New_York".to_owned()),
    };

    let after = Utc.with_ymd_and_hms(2026, 11, 1, 6, 10, 0).unwrap(); // 01:10 EST (second pass)
    let next = CronJob::compute_next_run(&schedule, after);
    assert!(next > after, "next_run must be in the future, got {next}");
    assert_eq!(next, Utc.with_ymd_and_hms(2026, 11, 1, 6, 30, 0).unwrap());

    // Once both occurrences have passed, the next firing is the next day.
    let late = Utc.with_ymd_and_hms(2026, 11, 1, 6, 40, 0).unwrap(); // 01:40 EST
    let next_day = CronJob::compute_next_run(&schedule, late);
    assert_eq!(
        next_day,
        Utc.with_ymd_and_hms(2026, 11, 2, 6, 30, 0).unwrap()
    );
}

#[tokio::test]
async fn test_cron_dst_spring_forward_skips_nonexistent_wall_clock_time() {
    use chrono::TimeZone;

    // America/New_York springs forward on 2026-03-08: 02:30 local never
    // occurs that day, so the next firing is the following day's 02:30 EDT.
    let schedule = CronSchedule::Cron {
        expr: "0 30 2 * * *".to_owned(),
        timezone: Some("America/New_York".to_owned()),
    };

    let after = Utc.with_ymd_and_hms(2026, 3, 8, 6, 55, 0).unwrap(); // 01:55 EST
    let next = CronJob::compute_next_run(&schedule, after);
    assert_eq!(next, Utc.with_ymd_and_hms(2026, 3, 9, 6, 30, 0).unwrap());
}

#[test]
fn resolve_bare_cron_timezone_prefers_valid_tz_env_over_system_zone() {
    // A valid IANA TZ env wins.
    assert_eq!(
        resolve_bare_cron_timezone(Some("America/New_York"), Some("Asia/Shanghai")),
        Some(Tz::America__New_York)
    );
    // Legacy TZDB names like EST5EDT parse as real zones (with DST rules),
    // matching how chrono::Local would honor the same TZ value.
    assert_eq!(
        resolve_bare_cron_timezone(Some("EST5EDT"), Some("Asia/Shanghai")),
        Some(Tz::EST5EDT)
    );
    // Full POSIX transition specs are not TZDB names: fall through to the
    // system zone.
    assert_eq!(
        resolve_bare_cron_timezone(Some("EST5EDT,M3.2.0/2,M11.1.0/2"), Some("Asia/Shanghai")),
        Some(Tz::Asia__Shanghai)
    );
    // Blank env falls through; missing both resolves to None (caller then
    // falls back to chrono::Local).
    assert_eq!(
        resolve_bare_cron_timezone(Some("  "), Some("Asia/Shanghai")),
        Some(Tz::Asia__Shanghai)
    );
    assert_eq!(resolve_bare_cron_timezone(None, None), None);
    assert_eq!(resolve_bare_cron_timezone(Some("garbage"), None), None);
}

#[tokio::test]
async fn test_bare_cron_dst_fall_back_fires_first_occurrence_under_tz_env() {
    use chrono::TimeZone;

    // Guards the bare-cron (timezone: None) machine-timezone path across a
    // DST fall-back. Only meaningful when the whole process runs under
    // TZ=America/New_York (the review repro:
    // `TZ=America/New_York cargo test -p garyx-gateway --lib cron`);
    // in-test env mutation would race parallel tests, so skip otherwise.
    if std::env::var("TZ").as_deref() != Ok("America/New_York") {
        return;
    }

    let schedule = CronSchedule::Cron {
        expr: "0 30 1 * * *".to_owned(),
        timezone: None,
    };
    let after = Utc.with_ymd_and_hms(2026, 11, 1, 4, 55, 0).unwrap(); // 00:55 EDT
    let next = CronJob::compute_next_run(&schedule, after);
    assert_eq!(next, Utc.with_ymd_and_hms(2026, 11, 1, 5, 30, 0).unwrap());

    let rearmed = CronJob::compute_next_run(&schedule, next);
    assert_eq!(
        rearmed,
        Utc.with_ymd_and_hms(2026, 11, 2, 6, 30, 0).unwrap()
    );
}

#[tokio::test]
async fn test_config_merge_preserves_runtime_state() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    // Add a job and run it.
    svc.add(make_job_config("merge1", 60)).await.unwrap();
    svc.run_now("merge1", &bare_exec_env()).await.unwrap();

    // Reload with config that changes the interval.
    let cfg = garyx_models::config::CronConfig {
        jobs: vec![CronJobConfig {
            id: "merge1".to_owned(),
            kind: Default::default(),
            label: None,
            schedule: CronSchedule::Interval { interval_secs: 120 },
            ui_schedule: None,
            action: CronAction::Log,
            target: None,
            message: None,
            workspace_dir: None,
            agent_id: None,
            thread_id: None,
            delete_after_run: false,
            enabled: true,
            system: false,
        }],
    };
    svc.load(&cfg).await.unwrap();

    let jobs = svc.list().await;
    let j = jobs.iter().find(|j| j.id == "merge1").unwrap();
    // Schedule updated from config.
    assert_eq!(j.schedule, CronSchedule::Interval { interval_secs: 120 });
    // Run count preserved from runtime.
    assert_eq!(j.run_count, 1);
}

#[tokio::test]
async fn test_load_recomputes_next_run_after_schedule_change() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    svc.add(make_job_config("reload-next-run", 3600))
        .await
        .unwrap();
    let before = svc
        .get("reload-next-run")
        .await
        .expect("job before reload")
        .next_run;

    svc.load(&CronConfig {
        jobs: vec![CronJobConfig {
            id: "reload-next-run".to_owned(),
            kind: Default::default(),
            label: None,
            schedule: CronSchedule::Interval { interval_secs: 60 },
            ui_schedule: None,
            action: CronAction::Log,
            target: None,
            message: None,
            workspace_dir: None,
            agent_id: None,
            thread_id: None,
            delete_after_run: false,
            enabled: true,
            system: false,
        }],
    })
    .await
    .unwrap();

    let after = svc.get("reload-next-run").await.expect("job after reload");
    assert_eq!(after.schedule, CronSchedule::Interval { interval_secs: 60 });
    assert!(after.next_run < before);
}

#[tokio::test]
async fn test_start_stop_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    svc.start(bare_exec_env());
    // Let it tick a couple times.
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    svc.stop().await;
    // Should not panic or hang.
}

#[tokio::test]
async fn test_stop_waits_for_inflight_cron_tick() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    svc.add(CronJobConfig {
        id: "slow-stop".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Slow Stop".to_owned()),
        schedule: CronSchedule::Interval { interval_secs: 0 },
        ui_schedule: None,
        action: CronAction::AgentTurn,
        target: None,
        message: Some("sleep".to_owned()),
        workspace_dir: Some("/tmp/slow-stop".to_owned()),
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .unwrap();

    let store: Arc<dyn ThreadStore> = Arc::new(garyx_router::InMemoryThreadStore::new());
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("automation-success", Arc::new(SuccessfulAutomationProvider))
        .await;

    // A port whose front-door dispatch blocks until released: the tick that
    // reaches it stays in flight, so stop() must wait for that tick.
    struct BlockingPort {
        gate: Arc<tokio::sync::Semaphore>,
    }
    #[async_trait]
    impl AutomationDispatchPort for BlockingPort {
        fn provider_runtime_ready(&self) -> bool {
            true
        }
        async fn invalidate_gateway_sync_caches(&self) {}
        async fn dispatch_internal_message(
            &self,
            _thread_id: &str,
            _run_id: &str,
            _message: &str,
            _extra_metadata: std::collections::HashMap<String, serde_json::Value>,
        ) -> Result<garyx_models::provider::AgentDispatchOutcome, AutomationDispatchError>
        {
            let _permit = self.gate.acquire().await.expect("gate closed");
            Err(AutomationDispatchError::StateUnavailable)
        }
    }
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let mut env = stateless_exec_env_with(store, bridge, true);
    env.port = Arc::new(BlockingPort { gate: gate.clone() });

    svc.start(env);
    tokio::time::sleep(tokio::time::Duration::from_millis(120)).await;
    {
        let mut stop_future = std::pin::pin!(svc.stop());
        let stop_early = tokio::select! {
            _ = &mut stop_future => true,
            _ = tokio::time::sleep(tokio::time::Duration::from_millis(50)) => false,
        };
        assert!(!stop_early);

        gate.add_permits(1);
        stop_future.as_mut().await;
    }
    assert!(svc.stop_tx.lock().unwrap().is_none());
    assert!(svc.scheduler_task.lock().unwrap().is_none());
}

#[tokio::test]
async fn test_tick_and_run_now_do_not_execute_same_job_twice() {
    let tmp = TempDir::new().unwrap();
    let svc = Arc::new(CronService::new(tmp.path().to_path_buf()));
    let _ = ensure_dirs(tmp.path()).await;

    svc.add(CronJobConfig {
        id: "single-flight".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Single Flight".to_owned()),
        schedule: CronSchedule::Interval { interval_secs: 0 },
        ui_schedule: None,
        action: CronAction::AgentTurn,
        target: None,
        message: Some("race".to_owned()),
        workspace_dir: Some("/tmp/single-flight".to_owned()),
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .unwrap();

    let bridge = Arc::new(MultiProviderBridge::new());
    let provider = Arc::new(CountingAutomationProvider::new(150));
    bridge
        .register_provider("counting-automation", provider.clone())
        .await;
    bridge.set_default_provider_key("counting-automation").await;
    let (_state, env) = wire_front_door_state(&svc, bridge).await;

    let run_now_task = {
        let svc = svc.clone();
        let env = env.clone();
        tokio::spawn(async move { svc.run_now("single-flight", &env).await })
    };
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    CronService::tick(
        &svc.jobs,
        &svc.runs,
        &svc.active_agent_runs,
        tmp.path(),
        &env,
    )
    .await;
    let _ = run_now_task.await.unwrap();

    assert_eq!(provider.calls(), 1);
}

#[tokio::test]
async fn test_start_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    svc.start(bare_exec_env());
    assert!(svc.stop_tx.lock().unwrap().is_some());

    // Second start should be ignored and keep current run loop.
    svc.start(bare_exec_env());
    assert!(svc.stop_tx.lock().unwrap().is_some());

    svc.stop().await;
    assert!(svc.stop_tx.lock().unwrap().is_none());
}

// ---------------------------------------------------------------------------
// AXON-687: schedule_followup + InternalDispatch wiring
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_legacy_cron_job_json_without_system_deserializes() {
    // Persisted state written by versions of the gateway that predate the
    // `system` field must still round-trip. Mirrors the on-disk schema
    // before AXON-687 — note there is no `system` field at all.
    let legacy = serde_json::json!({
        "id": "legacy-job",
        "kind": "automation_prompt",
        "schedule": { "type": "interval", "interval_secs": 60 },
        "action": "log",
        "delete_after_run": false,
        "enabled": true,
        "next_run": "2026-05-26T00:00:00Z",
        "last_status": "never_run",
        "created_at": "2026-05-26T00:00:00Z"
    });
    let job: CronJob = serde_json::from_value(legacy).expect("legacy CronJob json must parse");
    assert!(!job.system, "missing `system` field must default to false");
    assert_eq!(job.id, "legacy-job");
    assert_eq!(job.kind, CronJobKind::AutomationPrompt);
}

#[tokio::test]
async fn test_internal_dispatch_kind_serde_roundtrip() {
    let payload = garyx_models::config::InternalDispatchJobPayload {
        prompt: "continue with the report".to_owned(),
        reason: Some("background build finished".to_owned()),
        originating_run_id: Some("run-abc".to_owned()),
        scheduled_at: Utc::now(),
        delay_seconds_requested: 300,
    };
    let kind = CronJobKind::InternalDispatch {
        payload: payload.clone(),
    };
    let json = serde_json::to_value(&kind).unwrap();
    // Externally tagged: {"internal_dispatch": { ... }}
    assert!(
        json.get("internal_dispatch").is_some(),
        "InternalDispatch must serialize externally-tagged: {json}"
    );
    let parsed: CronJobKind = serde_json::from_value(json).unwrap();
    assert_eq!(parsed, kind);

    // And AutomationPrompt must still serialize as a bare string for
    // backwards compatibility.
    let auto = CronJobKind::AutomationPrompt;
    let json = serde_json::to_value(&auto).unwrap();
    assert_eq!(json, serde_json::json!("automation_prompt"));
}

#[tokio::test]
async fn test_list_hides_system_jobs_by_default() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());

    svc.add(CronJobConfig {
        id: "user-visible".to_owned(),
        kind: Default::default(),
        label: None,
        schedule: CronSchedule::Interval { interval_secs: 60 },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .unwrap();

    svc.add(CronJobConfig {
        id: "hidden-system".to_owned(),
        kind: Default::default(),
        label: None,
        schedule: CronSchedule::Interval { interval_secs: 60 },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: true,
    })
    .await
    .unwrap();

    let visible_ids: Vec<String> = svc.list().await.into_iter().map(|j| j.id).collect();
    assert_eq!(visible_ids, vec!["user-visible".to_owned()]);

    let all_ids: std::collections::BTreeSet<String> =
        svc.list_all().await.into_iter().map(|j| j.id).collect();
    let expected: std::collections::BTreeSet<String> = ["user-visible", "hidden-system"]
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
    assert_eq!(all_ids, expected);

    // `get` still returns system jobs by id — the filter is list-only.
    assert!(svc.get("hidden-system").await.is_some());
}

#[tokio::test]
async fn test_upsert_returns_replaced_previous() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());

    let make_cfg = |delay: u64| CronJobConfig {
        id: "dedupe-target".to_owned(),
        kind: CronJobKind::InternalDispatch {
            payload: garyx_models::config::InternalDispatchJobPayload {
                prompt: format!("delay={delay}"),
                reason: None,
                originating_run_id: Some("run-1".to_owned()),
                scheduled_at: Utc::now(),
                delay_seconds_requested: delay,
            },
        },
        label: None,
        schedule: CronSchedule::Once {
            at: (Utc::now() + chrono::Duration::seconds(delay as i64)).to_rfc3339(),
        },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: Some("thread::test".to_owned()),
        delete_after_run: true,
        enabled: true,
        system: true,
    };

    let (first, replaced_first) = svc.upsert(make_cfg(60)).await.unwrap();
    assert!(
        replaced_first.is_none(),
        "first upsert should not report a previous"
    );
    assert_eq!(first.id, "dedupe-target");

    let (second, replaced_second) = svc.upsert(make_cfg(120)).await.unwrap();
    assert_eq!(second.id, "dedupe-target");
    let prev = replaced_second.expect("second upsert must surface the replaced job");
    assert_eq!(prev.id, "dedupe-target");
    match prev.kind {
        CronJobKind::InternalDispatch { payload } => {
            assert_eq!(payload.delay_seconds_requested, 60);
        }
        _ => panic!("previous kind should be InternalDispatch"),
    }
}

#[test]
fn test_build_followup_body_contains_metadata_block() {
    let scheduled_at = chrono::DateTime::parse_from_rfc3339("2026-05-26T07:25:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let scheduled_for = chrono::DateTime::parse_from_rfc3339("2026-05-26T07:30:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let payload = garyx_models::config::InternalDispatchJobPayload {
        prompt: "resume the export".to_owned(),
        reason: Some("background job completed".to_owned()),
        originating_run_id: Some("run-xyz".to_owned()),
        scheduled_at,
        delay_seconds_requested: 300,
    };
    let body =
        crate::cron::build_followup_body("followup_deadbeefdeadbeef", &payload, scheduled_for);
    assert!(body.starts_with("<garyx_followup_metadata>"));
    assert!(body.contains("schedule_id: followup_deadbeefdeadbeef"));
    assert!(body.contains("delay_seconds_requested: 300"));
    // Agent-facing timestamps render as machine-local wall-clock time
    // (`YYYY-MM-DD HH:MM:SS`, timezone implicit).
    assert!(body.contains(&format!(
        "scheduled_at: {}",
        scheduled_at.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S")
    )));
    assert!(body.contains(&format!(
        "scheduled_for: {}",
        scheduled_for.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S")
    )));
    assert!(body.contains("reason: background job completed"));
    assert!(body.contains("originating_run_id: run-xyz"));
    assert!(body.contains("</garyx_followup_metadata>"));
    // Verbatim prompt must follow the metadata block on its own paragraph.
    assert!(body.contains("\n\nresume the export"));
}

#[tokio::test]
async fn test_followup_job_id_is_deterministic_and_distinguishes_runs() {
    // Same (thread, run) → same job id (dedupe key); different run → different id.
    use crate::mcp::tools::schedule_followup::followup_job_id;
    let a = followup_job_id("thread::abc", "run-1");
    let b = followup_job_id("thread::abc", "run-1");
    let c = followup_job_id("thread::abc", "run-2");
    let d = followup_job_id("thread::other", "run-1");
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_ne!(a, d);
    assert!(a.starts_with("followup_"));
}

#[tokio::test]
async fn test_internal_dispatch_followup_fires_and_injects_synthetic_user_turn() {
    // End-to-end: schedule a delay-soon followup via `upsert`, drive the
    // cron tick manually, and assert that the bridge provider sees a
    // synthetic user turn whose body begins with the
    // `<garyx_followup_metadata>` block. This is the AXON-687 acceptance
    // criterion that closes the schedule→tick→dispatch loop.
    use crate::composition::app_bootstrap::AppStateBuilder;
    use crate::mcp::tools::schedule_followup::followup_job_id;
    use garyx_bridge::{BridgeError, ProviderRuntime};
    use garyx_models::config::GaryxConfig;
    use garyx_models::provider::{
        ProviderRunOptions, ProviderRunResult, ProviderType, StreamEvent,
    };
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct RecordingProvider {
        calls: StdMutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl ProviderRuntime for RecordingProvider {
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
            self.calls
                .lock()
                .unwrap()
                .push((options.thread_id.clone(), options.message.clone()));
            on_chunk(StreamEvent::Done);
            Ok(ProviderRunResult {
                run_id: "ok".to_owned(),
                thread_id: options.thread_id.clone(),
                response: "ok".to_owned(),
                session_messages: vec![],
                sdk_session_id: None,
                actual_model: None,
                thread_title: None,
                success: true,
                error: None,
                input_tokens: 0,
                output_tokens: 0,
                cost: 0.0,
                duration_ms: 0,
            })
        }
        async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
            Ok(session_key.to_owned())
        }
    }

    let tmp = TempDir::new().unwrap();
    let cron = Arc::new(CronService::new(tmp.path().to_path_buf()));

    let bridge = Arc::new(garyx_bridge::MultiProviderBridge::new());
    let provider = Arc::new(RecordingProvider::default());
    bridge
        .register_provider("axon-687-provider", provider.clone())
        .await;
    bridge
        .set_route("telegram", "bot1", "axon-687-provider")
        .await;
    bridge.set_default_provider_key("axon-687-provider").await;

    let state = AppStateBuilder::new(GaryxConfig::default())
        .with_bridge(bridge.clone())
        .with_cron_service(cron.clone())
        .with_custom_agent_store(Arc::new(crate::custom_agents::CustomAgentStore::new()))
        .build();
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    bridge.set_event_tx(state.ops.events.sender()).await;

    // Seed a thread with the channel binding so dispatch_internal_message
    // can resolve a delivery context. Synthetic placeholder ids only —
    // never real chat ids.
    let thread_id = "thread::axon687-followup";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            serde_json::json!({
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
                    "display_label": "test-user"
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

    // Schedule an internal-dispatch job that should fire in ~500ms.
    // Bypass the MCP tool's 60s minimum on purpose — this test runs in CI
    // and we want the tick to fire deterministically, not wait a minute.
    let run_id = "run-from-test";
    let job_id = followup_job_id(thread_id, run_id);
    let scheduled_at = Utc::now();
    let scheduled_for = scheduled_at + chrono::Duration::milliseconds(500);
    let payload = garyx_models::config::InternalDispatchJobPayload {
        prompt: "resume verification".to_owned(),
        reason: Some("test reason".to_owned()),
        originating_run_id: Some(run_id.to_owned()),
        scheduled_at,
        delay_seconds_requested: 60,
    };
    cron.upsert(CronJobConfig {
        id: job_id.clone(),
        kind: CronJobKind::InternalDispatch {
            payload: payload.clone(),
        },
        label: None,
        schedule: CronSchedule::Once {
            at: scheduled_for.to_rfc3339(),
        },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: Some(thread_id.to_owned()),
        delete_after_run: true,
        enabled: true,
        system: true,
    })
    .await
    .unwrap();

    // Wait past the firing window, then drive the tick directly.
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    CronService::tick(
        &cron.jobs,
        &cron.runs,
        &cron.active_agent_runs,
        tmp.path(),
        &crate::composition::automation_wiring::automation_exec_env(&state),
    )
    .await;

    // Allow the bridge's spawn-and-stream path to land.
    let calls = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let snapshot = provider.calls.lock().unwrap().clone();
            if !snapshot.is_empty() {
                break snapshot;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("provider should receive the synthetic followup user turn");

    assert_eq!(calls.len(), 1, "exactly one synthetic dispatch expected");
    assert_eq!(calls[0].0, thread_id);
    let body = &calls[0].1;
    assert!(
        body.starts_with("<garyx_followup_metadata>"),
        "body must lead with metadata block, got: {body}"
    );
    assert!(body.contains(&format!("schedule_id: {job_id}")));
    assert!(body.contains("delay_seconds_requested: 60"));
    assert!(body.contains("reason: test reason"));
    assert!(body.contains("originating_run_id: run-from-test"));
    assert!(
        body.contains("\n\nresume verification"),
        "verbatim prompt must follow the metadata block"
    );
}

// ---------------------------------------------------------------------------
// schedule_followup boundary fallback — drop classification + retry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_followup_retry_drops_immediately_without_retry() {
    use std::sync::atomic::{AtomicU32, Ordering};

    let calls = AtomicU32::new(0);
    let result = CronService::run_followup_with_retry(
        FOLLOWUP_MAX_RETRIES,
        std::time::Duration::ZERO,
        "job-drop",
        "run-drop",
        |_attempt| {
            calls.fetch_add(1, Ordering::SeqCst);
            async {
                Err(FollowupAttemptError::Dropped(
                    "thread not found: t1".to_owned(),
                ))
            }
        },
    )
    .await;

    assert_eq!(
        result.unwrap_err(),
        "thread not found: t1",
        "non-retryable drop returns its reason verbatim"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a Dropped outcome must not be retried"
    );
}

#[tokio::test]
async fn test_followup_retry_succeeds_after_transient_failures() {
    use std::sync::atomic::{AtomicU32, Ordering};

    let calls = AtomicU32::new(0);
    let result = CronService::run_followup_with_retry(
        FOLLOWUP_MAX_RETRIES,
        std::time::Duration::ZERO,
        "job-ok",
        "run-ok",
        |_attempt| {
            let c = calls.fetch_add(1, Ordering::SeqCst);
            async move {
                if c < 2 {
                    Err(FollowupAttemptError::Transient(format!("boom {c}")))
                } else {
                    Ok(())
                }
            }
        },
    )
    .await;

    assert!(
        result.is_ok(),
        "transient failures within budget then success must succeed: {result:?}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "two transient failures then a successful third attempt"
    );
}

#[tokio::test]
async fn test_followup_retry_exhausts_budget_and_drops() {
    use std::sync::atomic::{AtomicU32, Ordering};

    let calls = AtomicU32::new(0);
    let result = CronService::run_followup_with_retry(
        FOLLOWUP_MAX_RETRIES,
        std::time::Duration::ZERO,
        "job-exhaust",
        "run-exhaust",
        |_attempt| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(FollowupAttemptError::Transient("network down".to_owned())) }
        },
    )
    .await;

    let err = result.unwrap_err();
    assert!(
        err.contains(&format!("after {FOLLOWUP_MAX_RETRIES} retries")),
        "exhausted error names the retry count, got: {err}"
    );
    assert!(
        err.contains("network down"),
        "exhausted error carries the concrete underlying failure, got: {err}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        FOLLOWUP_MAX_RETRIES + 1,
        "one initial attempt plus FOLLOWUP_MAX_RETRIES retries"
    );
}

#[tokio::test]
async fn test_internal_dispatch_drops_when_thread_missing() {
    use garyx_models::config::GaryxConfig;

    let tmp = TempDir::new().unwrap();
    let cron = Arc::new(CronService::new(tmp.path().to_path_buf()));
    let _ = ensure_dirs(tmp.path()).await;

    let bridge = Arc::new(garyx_bridge::MultiProviderBridge::new());
    // No thread is seeded: the originating thread was deleted before the
    // followup fired. The pre-check short-circuits before any provider call.
    let state = crate::server::AppStateBuilder::new(GaryxConfig::default())
        .with_bridge(bridge.clone())
        .with_cron_service(cron.clone())
        .with_custom_agent_store(Arc::new(crate::custom_agents::CustomAgentStore::new()))
        .build();
    // Keep the builder result alive for the duration of the tick so the
    // execution env's dispatch port can upgrade its state handle.
    let _state = state;

    let thread_id = "thread::followup-deleted-target";
    let run_id = "run-from-test";
    let job_id = crate::mcp::tools::schedule_followup::followup_job_id(thread_id, run_id);
    let scheduled_at = Utc::now();
    let scheduled_for = scheduled_at + chrono::Duration::milliseconds(200);
    let payload = garyx_models::config::InternalDispatchJobPayload {
        prompt: "resume verification".to_owned(),
        reason: Some("test reason".to_owned()),
        originating_run_id: Some(run_id.to_owned()),
        scheduled_at,
        delay_seconds_requested: 60,
    };
    cron.upsert(CronJobConfig {
        id: job_id.clone(),
        kind: CronJobKind::InternalDispatch {
            payload: payload.clone(),
        },
        label: None,
        schedule: CronSchedule::Once {
            at: scheduled_for.to_rfc3339(),
        },
        ui_schedule: None,
        action: CronAction::Log,
        target: None,
        message: None,
        workspace_dir: None,
        agent_id: None,
        thread_id: Some(thread_id.to_owned()),
        delete_after_run: true,
        enabled: true,
        system: true,
    })
    .await
    .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    CronService::tick(
        &cron.jobs,
        &cron.runs,
        &cron.active_agent_runs,
        tmp.path(),
        &crate::composition::automation_wiring::automation_exec_env(&_state),
    )
    .await;

    let runs = cron.list_runs(10, 0).await;
    assert_eq!(runs.len(), 1, "exactly one run should be recorded");
    assert_eq!(
        runs[0].status,
        JobRunStatus::FailedDropped,
        "a deleted thread must produce a dropped run, got: {:?}",
        runs[0].status
    );
    let reason = runs[0].error.as_deref().unwrap_or_default();
    assert!(
        reason.contains("thread not found"),
        "drop reason must explain the missing thread, got: {reason:?}"
    );
    // The persisted (serde) form must be exactly "failed_dropped" per AC.
    let serialized = serde_json::to_string(&runs[0].status).unwrap();
    assert_eq!(serialized, "\"failed_dropped\"");
}

fn structurally_invalid_job_config(id: &str, action: CronAction, enabled: bool) -> CronJobConfig {
    CronJobConfig {
        id: id.to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Repairable invalid job".to_owned()),
        schedule: CronSchedule::Interval { interval_secs: 60 },
        ui_schedule: None,
        action,
        target: None,
        message: Some("Run later".to_owned()),
        workspace_dir: None,
        agent_id: Some("claude".to_owned()),
        thread_id: None,
        delete_after_run: false,
        enabled,
        system: false,
    }
}

#[test]
fn noncanonical_thread_id_cannot_hide_behind_generated_workspace_or_delivery_target() {
    let mut agent_turn =
        structurally_invalid_job_config("invalid-thread-agent", CronAction::AgentTurn, true);
    agent_turn.workspace_dir = Some("/tmp/generated-but-invalid-thread".to_owned());
    agent_turn.thread_id = Some("cron::not-a-thread".to_owned());
    agent_turn.target = Some("last".to_owned());
    assert_eq!(
        CronJob::from_config(&agent_turn)
            .validation_error
            .as_deref(),
        Some("invalid canonical thread_id for agent turn")
    );

    let mut system_event =
        structurally_invalid_job_config("invalid-thread-system", CronAction::SystemEvent, true);
    system_event.thread_id = Some("cron::not-a-thread".to_owned());
    system_event.target = Some("last".to_owned());
    assert_eq!(
        CronJob::from_config(&system_event)
            .validation_error
            .as_deref(),
        Some("invalid canonical thread_id for system event")
    );
}

#[tokio::test]
async fn validation_recomputes_invalid_valid_invalid_without_restart_and_is_not_persisted() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let invalid =
        structurally_invalid_job_config("validation-transition", CronAction::AgentTurn, true);
    let added = svc.add(invalid.clone()).await.unwrap();
    assert_eq!(
        added.validation_error.as_deref(),
        Some("missing canonical target for agent turn")
    );

    let first_run = svc.run_now("validation-transition", &bare_exec_env()).await.unwrap();
    assert_eq!(first_run.status, JobRunStatus::Failed);
    assert_eq!(
        first_run.error.as_deref(),
        Some("missing canonical target for agent turn")
    );

    let mut valid = invalid.clone();
    valid.workspace_dir = Some("/tmp/automation-validation".to_owned());
    valid.agent_id = None;
    let valid = svc
        .update("validation-transition", valid)
        .await
        .unwrap()
        .unwrap();
    assert!(valid.validation_error.is_none());
    assert_eq!(valid.agent_id.as_deref(), Some("claude"));

    let invalid_again = svc
        .update("validation-transition", invalid)
        .await
        .unwrap()
        .unwrap();
    assert!(invalid_again.validation_error.is_some());
    let persisted =
        std::fs::read_to_string(jobs_dir(tmp.path()).join("validation-transition.json")).unwrap();
    assert!(!persisted.contains("validation_error"));

    let mut upsert_valid =
        structurally_invalid_job_config("validation-upsert", CronAction::AgentTurn, true);
    upsert_valid.workspace_dir = Some("/tmp/automation-validation".to_owned());
    let (upserted, _) = svc.upsert(upsert_valid).await.unwrap();
    assert!(upserted.validation_error.is_none());
    let (upserted_invalid, _) = svc
        .upsert(structurally_invalid_job_config(
            "validation-upsert",
            CronAction::AgentTurn,
            true,
        ))
        .await
        .unwrap();
    assert!(upserted_invalid.validation_error.is_some());
}

#[tokio::test]
async fn scheduler_claim_revalidates_stale_validation_fail_closed() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    svc.add(structurally_invalid_job_config(
        "stale-validation",
        CronAction::AgentTurn,
        true,
    ))
    .await
    .unwrap();
    {
        let mut jobs = svc.jobs.write().await;
        let job = jobs.get_mut("stale-validation").unwrap();
        job.validation_error = None;
        job.next_run = Utc::now() - chrono::Duration::seconds(1);
    }

    let claimed = CronService::claim_job_for_execution(
        tmp.path(),
        &svc.jobs,
        &svc.active_agent_runs,
        &bare_exec_env(),
        "stale-validation",
    )
    .await;
    assert!(claimed.is_none());
    assert_eq!(
        svc.get("stale-validation")
            .await
            .unwrap()
            .validation_error
            .as_deref(),
        Some("missing canonical target for agent turn")
    );
    assert!(
        svc.list_runs_for_job("stale-validation", 10, 0)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn load_marks_threadless_agent_turn_and_system_event_invalid_from_disk_and_config() {
    for availability in ["enabled", "selected-disabled", "all-disabled"] {
        let tmp = TempDir::new().unwrap();
        ensure_dirs(tmp.path()).await.unwrap();
        for (id, action, enabled) in [
            ("disk-agent", CronAction::AgentTurn, true),
            ("disk-system", CronAction::SystemEvent, false),
        ] {
            let job = CronJob::from_config(&structurally_invalid_job_config(id, action, enabled));
            persist_job(tmp.path(), &job).await.unwrap();
        }
        let config = CronConfig {
            jobs: vec![
                structurally_invalid_job_config("config-agent", CronAction::AgentTurn, false),
                structurally_invalid_job_config("config-system", CronAction::SystemEvent, true),
            ],
        };
        let svc = Arc::new(CronService::new(tmp.path().to_path_buf()));
        svc.load(&config).await.unwrap();

        let custom_agents = Arc::new(crate::custom_agents::CustomAgentStore::new());
        match availability {
            "enabled" => {}
            "selected-disabled" => {
                custom_agents
                    .set_enabled("claude", false)
                    .await
                    .expect("disable selected agent");
                assert_eq!(
                    custom_agents.effective_default_agent_id().await.as_deref(),
                    Some("codex"),
                    "the structural rejection must not be a no-enabled false positive"
                );
            }
            "all-disabled" => {
                for agent in custom_agents.list_agents().await {
                    custom_agents
                        .set_enabled(&agent.agent_id, false)
                        .await
                        .expect("disable every agent");
                }
                assert!(custom_agents.effective_default_agent_id().await.is_none());
            }
            _ => unreachable!(),
        }
        let provider = Arc::new(CountingAutomationProvider::new(0));
        let bridge = Arc::new(MultiProviderBridge::new());
        bridge
            .register_provider("threadless-validation-provider", provider.clone())
            .await;
        bridge
            .set_default_provider_key("threadless-validation-provider")
            .await;
        let (_state, env) = wire_front_door_state_with_agents(&svc, bridge, custom_agents.clone()).await;

        for (id, expected) in [
            ("disk-agent", "missing canonical target for agent turn"),
            ("disk-system", "missing canonical target for system event"),
            ("config-agent", "missing canonical target for agent turn"),
            ("config-system", "missing canonical target for system event"),
        ] {
            let job = svc.get(id).await.unwrap();
            assert_eq!(
                job.validation_error.as_deref(),
                Some(expected),
                "availability={availability} id={id}"
            );
            let run = svc
                .run_now(id, &env)
                .await
                .expect("invalid run is visibly recorded");
            assert_eq!(run.status, JobRunStatus::Failed);
            assert_eq!(run.error.as_deref(), Some(expected));
        }
        assert_eq!(
            provider.calls(),
            0,
            "thread-less jobs must never reach bridge/provider when availability={availability}"
        );
    }
}

#[tokio::test]
async fn generated_agent_normalization_is_idempotent_across_config_merge_reloads() {
    let tmp = TempDir::new().unwrap();
    let mut generated =
        structurally_invalid_job_config("generated-normalization", CronAction::AgentTurn, true);
    generated.workspace_dir = Some("/tmp/generated-normalization".to_owned());
    generated.agent_id = Some("   ".to_owned());
    let config = CronConfig {
        jobs: vec![generated.clone()],
    };

    let first = CronService::new(tmp.path().to_path_buf());
    first.load(&config).await.unwrap();
    assert_eq!(
        first
            .get("generated-normalization")
            .await
            .unwrap()
            .agent_id
            .as_deref(),
        Some("claude")
    );

    let second = CronService::new(tmp.path().to_path_buf());
    second.load(&config).await.unwrap();
    assert_eq!(
        second
            .get("generated-normalization")
            .await
            .unwrap()
            .agent_id
            .as_deref(),
        Some("claude")
    );
    assert_eq!(generated.agent_id.as_deref(), Some("   "));
}

#[test]
fn log_and_internal_dispatch_jobs_are_outside_agent_validation_contract() {
    let mut log = CronJob::from_config(&make_job_config("log-unaffected", 60));
    log.action = CronAction::Log;
    assert!(validate_cron_job(&log).is_none());

    let internal = CronJob::from_config(&CronJobConfig {
        id: "internal-unaffected".to_owned(),
        kind: CronJobKind::InternalDispatch {
            payload: garyx_models::config::InternalDispatchJobPayload {
                prompt: "continue".to_owned(),
                scheduled_at: Utc::now(),
                delay_seconds_requested: 1,
                reason: None,
                originating_run_id: None,
            },
        },
        action: CronAction::AgentTurn,
        ..make_job_config("internal-unaffected", 60)
    });
    assert!(validate_cron_job(&internal).is_none());
}
