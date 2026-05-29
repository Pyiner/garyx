use super::*;
use async_trait::async_trait;
use garyx_bridge::{AgentLoopProvider, BridgeError};
use garyx_channels::{ChannelDispatcher, ChannelDispatcherImpl, ChannelInfo, OutboundMessage};
use garyx_models::config::{CronAction, CronConfig, CronJobConfig, CronSchedule};
use garyx_models::provider::{
    ProviderRunOptions, ProviderRunResult, ProviderType, StreamBoundaryKind, StreamEvent,
};
use garyx_models::thread_logs::NoopThreadLogSink;
use garyx_router::ThreadStore;
use tempfile::TempDir;

#[derive(Default)]
struct RecordingDispatcher {
    calls: std::sync::Mutex<Vec<OutboundMessage>>,
    message_ids: std::sync::Mutex<Vec<String>>,
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
}

#[async_trait]
impl AgentLoopProvider for SuccessfulAutomationProvider {
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
        Self {
            calls: std::sync::atomic::AtomicUsize::new(0),
            delay_ms,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl AgentLoopProvider for CountingAutomationProvider {
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
    let record = svc.run_now("rn1").await.unwrap();
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

    let record = svc.run_now("rn-advance").await.unwrap();
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

    let record = svc.run_now("rn-once").await.unwrap();
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
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    svc.add(CronJobConfig {
        id: "tick-fail".to_owned(),
        kind: Default::default(),
        label: None,
        schedule: CronSchedule::Interval { interval_secs: 0 },
        ui_schedule: None,
        action: CronAction::AgentTurn,
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
        None,
        &svc.dispatch_runtime,
        &svc.app_state_weak,
        &svc.garyx_db,
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

    let record = svc.run_now("delete-now").await.unwrap();
    assert_eq!(record.status, JobRunStatus::Success);

    let jobs = svc.list().await;
    assert!(jobs.iter().all(|j| j.id != "delete-now"));
    assert!(!jobs_dir(tmp.path()).join("delete-now.json").exists());
}

#[tokio::test]
async fn test_build_scheduled_response_callback_sends_final_message() {
    let dispatcher = Arc::new(RecordingDispatcher::default());
    let router = Arc::new(tokio::sync::Mutex::new(MessageRouter::new(
        Arc::new(garyx_router::InMemoryThreadStore::new()),
        garyx_models::config::GaryxConfig::default(),
    )));
    let callback = build_scheduled_response_callback(
        dispatcher.clone(),
        router,
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
async fn test_build_scheduled_response_callback_records_reply_routing() {
    let dispatcher = Arc::new(RecordingDispatcher::with_message_ids(vec![
        "cron_msg_1".to_owned(),
    ]));
    let store: Arc<dyn ThreadStore> = Arc::new(garyx_router::InMemoryThreadStore::new());
    store
        .set(
            "cron::daily",
            serde_json::json!({
                "thread_id": "cron::daily",
            }),
        )
        .await;
    let router = Arc::new(tokio::sync::Mutex::new(MessageRouter::new(
        store.clone(),
        garyx_models::config::GaryxConfig::default(),
    )));
    let callback = build_scheduled_response_callback(
        dispatcher,
        router.clone(),
        ScheduledResponseContext {
            thread_id: "cron::daily".to_owned(),
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            chat_id: "42".to_owned(),
            delivery_target_type: "chat_id".to_owned(),
            delivery_target_id: "42".to_owned(),
            delivery_thread_id: Some("42_t100".to_owned()),
            thread_log_id: None,
        },
    );

    callback(StreamEvent::Delta {
        text: "ping".to_owned(),
    });
    callback(StreamEvent::Done);
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

    {
        let router_guard = router.lock().await;
        assert_eq!(
            router_guard.resolve_reply_thread_for_chat(
                "telegram",
                "main",
                Some("42"),
                Some("42_t100"),
                "cron_msg_1",
            ),
            Some("cron::daily")
        );
    }

    let thread_state = store.get("cron::daily").await.unwrap();
    assert_eq!(
        thread_state["outbound_message_ids"][0]["thread_binding_key"],
        serde_json::json!("42_t100")
    );
    assert_eq!(
        thread_state["outbound_message_ids"][0]["message_id"],
        serde_json::json!("cron_msg_1")
    );
}

#[tokio::test]
async fn test_build_scheduled_response_callback_preserves_assistant_segments() {
    let dispatcher = Arc::new(RecordingDispatcher::with_message_ids(vec![
        "cron_msg_1".to_owned(),
    ]));
    let store: Arc<dyn ThreadStore> = Arc::new(garyx_router::InMemoryThreadStore::new());
    let router = Arc::new(tokio::sync::Mutex::new(MessageRouter::new(
        store,
        garyx_models::config::GaryxConfig::default(),
    )));
    let callback = build_scheduled_response_callback(
        dispatcher.clone(),
        router,
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
    let store: Arc<dyn ThreadStore> = Arc::new(garyx_router::InMemoryThreadStore::new());
    let router = Arc::new(tokio::sync::Mutex::new(MessageRouter::new(
        store,
        garyx_models::config::GaryxConfig::default(),
    )));
    let callback = build_scheduled_response_callback(
        dispatcher.clone(),
        router,
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
        .await;
    let runtime = Arc::new(RwLock::new(Some(CronDispatchRuntime {
        thread_store: store.clone(),
        router: Arc::new(tokio::sync::Mutex::new(MessageRouter::new(
            store,
            garyx_models::config::GaryxConfig::default(),
        ))),
        bridge: Arc::new(MultiProviderBridge::new()),
        channel_dispatcher: Arc::new(ChannelDispatcherImpl::new()),
        thread_logs: Arc::new(NoopThreadLogSink),
        managed_mcp_servers: HashMap::new(),
        custom_agents: Arc::new(crate::custom_agents::CustomAgentStore::new()),
        agent_teams: Arc::new(crate::agent_teams::AgentTeamStore::new()),
    })));
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
    };

    let active_agent_runs = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    let err = CronService::dispatch_agent_turn(&job, "run-1", "ping", &active_agent_runs, &runtime)
        .await
        .expect_err("bridge has no providers in this test");
    assert!(err.contains("cron dispatch failed"));
    assert!(err.contains("channel=telegram"));
}

#[tokio::test]
async fn test_successful_automation_run_persists_thread_id() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
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

    let store: Arc<dyn ThreadStore> = Arc::new(garyx_router::InMemoryThreadStore::new());
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("automation-success", Arc::new(SuccessfulAutomationProvider))
        .await;
    svc.set_dispatch_runtime(
        store.clone(),
        Arc::new(tokio::sync::Mutex::new(MessageRouter::new(
            store,
            garyx_models::config::GaryxConfig::default(),
        ))),
        bridge,
        Arc::new(ChannelDispatcherImpl::new()),
        Arc::new(NoopThreadLogSink),
        HashMap::new(),
        Arc::new(crate::custom_agents::CustomAgentStore::new()),
        Arc::new(crate::agent_teams::AgentTeamStore::new()),
    )
    .await;

    let run = svc.run_now("automation-persist").await.unwrap();
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
async fn test_bound_automation_run_reuses_existing_thread() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
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

    let store: Arc<dyn ThreadStore> = Arc::new(garyx_router::InMemoryThreadStore::new());
    store
        .set(
            target_thread_id,
            serde_json::json!({
                "workspace_dir": "/tmp/bound-automation",
                "metadata": {
                    "agent_id": "claude"
                }
            }),
        )
        .await;
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("automation-success", Arc::new(SuccessfulAutomationProvider))
        .await;
    svc.set_dispatch_runtime(
        store.clone(),
        Arc::new(tokio::sync::Mutex::new(MessageRouter::new(
            store.clone(),
            garyx_models::config::GaryxConfig::default(),
        ))),
        bridge,
        Arc::new(ChannelDispatcherImpl::new()),
        Arc::new(NoopThreadLogSink),
        HashMap::new(),
        Arc::new(crate::custom_agents::CustomAgentStore::new()),
        Arc::new(crate::agent_teams::AgentTeamStore::new()),
    )
    .await;

    let run = svc.run_now("automation-bound").await.unwrap();
    assert_eq!(run.status, JobRunStatus::Success);
    assert_eq!(run.thread_id.as_deref(), Some(target_thread_id));

    let reloaded = CronService::new(tmp.path().to_path_buf());
    reloaded.load(&CronConfig::default()).await.unwrap();
    let reloaded_job = reloaded
        .get("automation-bound")
        .await
        .expect("reloaded automation job");

    assert_eq!(reloaded_job.thread_id.as_deref(), Some(target_thread_id));
    assert!(store.get(target_thread_id).await.is_some());
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
    svc.set_dispatch_runtime(
        store.clone(),
        Arc::new(tokio::sync::Mutex::new(MessageRouter::new(
            store.clone(),
            garyx_models::config::GaryxConfig::default(),
        ))),
        Arc::new(MultiProviderBridge::new()),
        Arc::new(ChannelDispatcherImpl::new()),
        Arc::new(NoopThreadLogSink),
        HashMap::new(),
        Arc::new(crate::custom_agents::CustomAgentStore::new()),
        Arc::new(crate::agent_teams::AgentTeamStore::new()),
    )
    .await;

    let run = svc.run_now("automation-missing-bound").await.unwrap();

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
    assert!(store.get(missing_thread_id).await.is_none());
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

    svc.set_dispatch_runtime(
        store.clone(),
        Arc::new(tokio::sync::Mutex::new(MessageRouter::new(
            store.clone(),
            garyx_models::config::GaryxConfig::default(),
        ))),
        Arc::new(MultiProviderBridge::new()),
        Arc::new(ChannelDispatcherImpl::new()),
        Arc::new(NoopThreadLogSink),
        HashMap::new(),
        Arc::new(crate::custom_agents::CustomAgentStore::new()),
        Arc::new(crate::agent_teams::AgentTeamStore::new()),
    )
    .await;

    let run = svc.run_now("automation-keep-thread").await.unwrap();
    assert_eq!(run.status, JobRunStatus::Failed);
    let failed_thread_id = run.thread_id.clone().expect("failed thread id");

    let job = svc
        .get("automation-keep-thread")
        .await
        .expect("automation job after failed run");
    assert!(job.thread_id.is_none());
    assert!(store.get(&failed_thread_id).await.is_none());
}

#[tokio::test]
async fn test_run_now_missing_job() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    assert!(svc.run_now("nonexistent").await.is_none());
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

    assert!(svc.run_now("disabled-now").await.is_none());
    assert!(svc.list_runs(10, 0).await.is_empty());
}

#[tokio::test]
async fn test_update_job_keeps_runtime_state() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    svc.add(make_job_config("upd1", 60)).await.unwrap();
    svc.run_now("upd1").await.unwrap();

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

    let utc_schedule = CronSchedule::Cron {
        expr: expr.clone(),
        timezone: None,
    };
    let shanghai_schedule = CronSchedule::Cron {
        expr,
        timezone: Some("Asia/Shanghai".to_owned()),
    };

    let next_utc = CronJob::compute_next_run(&utc_schedule, after);
    let next_shanghai = CronJob::compute_next_run(&shanghai_schedule, after);

    assert_eq!(next_utc, Utc.with_ymd_and_hms(2026, 1, 1, 9, 0, 0).unwrap());
    assert_eq!(
        next_shanghai,
        Utc.with_ymd_and_hms(2026, 1, 1, 1, 0, 0).unwrap()
    );
}

#[tokio::test]
async fn test_config_merge_preserves_runtime_state() {
    let tmp = TempDir::new().unwrap();
    let svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    // Add a job and run it.
    svc.add(make_job_config("merge1", 60)).await.unwrap();
    svc.run_now("merge1").await.unwrap();

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
    let mut svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    svc.start();
    // Let it tick a couple times.
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    svc.stop().await;
    // Should not panic or hang.
}

#[tokio::test]
async fn test_stop_waits_for_inflight_cron_tick() {
    let tmp = TempDir::new().unwrap();
    let mut svc = CronService::new(tmp.path().to_path_buf());
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
    svc.set_dispatch_runtime(
        store.clone(),
        Arc::new(tokio::sync::Mutex::new(MessageRouter::new(
            store,
            garyx_models::config::GaryxConfig::default(),
        ))),
        bridge,
        Arc::new(ChannelDispatcherImpl::new()),
        Arc::new(NoopThreadLogSink),
        HashMap::new(),
        Arc::new(crate::custom_agents::CustomAgentStore::new()),
        Arc::new(crate::agent_teams::AgentTeamStore::new()),
    )
    .await;

    let dispatch_runtime = svc.dispatch_runtime.clone();
    let dispatch_guard = dispatch_runtime.write().await;
    svc.start();
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    {
        let mut stop_future = std::pin::pin!(svc.stop());
        let stop_early = tokio::select! {
            _ = &mut stop_future => true,
            _ = tokio::time::sleep(tokio::time::Duration::from_millis(50)) => false,
        };
        assert!(!stop_early);

        drop(dispatch_guard);
        stop_future.as_mut().await;
    }
    assert!(svc.stop_tx.is_none());
    assert!(svc.scheduler_task.is_none());
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

    let store: Arc<dyn ThreadStore> = Arc::new(garyx_router::InMemoryThreadStore::new());
    let bridge = Arc::new(MultiProviderBridge::new());
    let provider = Arc::new(CountingAutomationProvider::new(150));
    bridge
        .register_provider("counting-automation", provider.clone())
        .await;
    svc.set_dispatch_runtime(
        store.clone(),
        Arc::new(tokio::sync::Mutex::new(MessageRouter::new(
            store,
            garyx_models::config::GaryxConfig::default(),
        ))),
        bridge,
        Arc::new(ChannelDispatcherImpl::new()),
        Arc::new(NoopThreadLogSink),
        HashMap::new(),
        Arc::new(crate::custom_agents::CustomAgentStore::new()),
        Arc::new(crate::agent_teams::AgentTeamStore::new()),
    )
    .await;

    let run_now_task = {
        let svc = svc.clone();
        tokio::spawn(async move { svc.run_now("single-flight").await })
    };
    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    CronService::tick(
        &svc.jobs,
        &svc.runs,
        &svc.active_agent_runs,
        tmp.path(),
        None,
        &svc.dispatch_runtime,
        &svc.app_state_weak,
        &svc.garyx_db,
    )
    .await;
    let _ = run_now_task.await.unwrap();

    assert_eq!(provider.calls(), 1);
}

#[tokio::test]
async fn test_start_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let mut svc = CronService::new(tmp.path().to_path_buf());
    let _ = ensure_dirs(tmp.path()).await;

    svc.start();
    let first_sender = svc.stop_tx.clone();
    assert!(first_sender.is_some());

    // Second start should be ignored and keep current run loop.
    svc.start();
    assert!(svc.stop_tx.is_some());

    svc.stop().await;
    assert!(svc.stop_tx.is_none());
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
    assert!(body.contains("scheduled_for: 2026-05-26T07:30:00+00:00"));
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
    use garyx_bridge::{AgentLoopProvider, BridgeError};
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
    impl AgentLoopProvider for RecordingProvider {
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
        .with_auto_research_store(Arc::new(crate::auto_research::AutoResearchStore::new()))
        .with_agent_team_store(Arc::new(crate::agent_teams::AgentTeamStore::new()))
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
        .await;

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
        None,
        &cron.dispatch_runtime,
        &cron.app_state_weak,
        &cron.garyx_db,
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
            async { Err(FollowupAttemptError::Dropped("thread not found: t1".to_owned())) }
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
        .with_auto_research_store(Arc::new(crate::auto_research::AutoResearchStore::new()))
        .with_agent_team_store(Arc::new(crate::agent_teams::AgentTeamStore::new()))
        .with_custom_agent_store(Arc::new(crate::custom_agents::CustomAgentStore::new()))
        .build();
    // Keep the builder result alive for the duration of the tick so the cron
    // service's weak app_state back-reference can upgrade.
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
        None,
        &cron.dispatch_runtime,
        &cron.app_state_weak,
        &cron.garyx_db,
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
