use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use chrono::{SecondsFormat, Utc};
use garyx_bridge::ClearSessionOutcome;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{Notify, watch};
use tokio::task::{JoinError, JoinHandle};

use crate::app_state::AppState;
use crate::garyx_db::{
    CleanupOutboxJob, CleanupOutboxStep, GaryxDbService, LifecycleOperationKind,
    LifecycleOperationRecord,
};

pub(crate) const LIFECYCLE_JOIN_WINDOW: Duration = Duration::from_secs(6);
const OUTBOX_IDLE_POLL: Duration = Duration::from_secs(1);
const OUTBOX_WARNING_ATTEMPT: u32 = 10;

#[cfg(any(test, feature = "test-seams"))]
struct LifecyclePauseState {
    started: AtomicBool,
    started_notify: Notify,
    released: AtomicBool,
    release_notify: Notify,
}

#[cfg(any(test, feature = "test-seams"))]
pub(crate) struct TestLifecyclePause {
    state: Arc<LifecyclePauseState>,
}

#[cfg(any(test, feature = "test-seams"))]
impl TestLifecyclePause {
    pub async fn wait_until_started(&self) {
        loop {
            if self.state.started.load(Ordering::Acquire) {
                return;
            }
            let notified = self.state.started_notify.notified();
            if self.state.started.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }

    pub fn release(&self) {
        self.state.released.store(true, Ordering::Release);
        self.state.release_notify.notify_waiters();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct OperationKey {
    pub store_incarnation: String,
    pub operation_id: String,
}

#[derive(Debug, Clone)]
pub(crate) enum OperationCellResult {
    Completed(LifecycleOperationRecord),
    OperationIdConflict,
    WrongIncarnation { current_store_incarnation: String },
    InProgress,
    TransientFailure,
}

struct OperationCell {
    fingerprint: String,
    result_tx: watch::Sender<Option<Arc<OperationCellResult>>>,
}

impl OperationCell {
    fn new(fingerprint: String) -> Arc<Self> {
        let (result_tx, _result_rx) = watch::channel(None);
        Arc::new(Self {
            fingerprint,
            result_tx,
        })
    }

    fn publish(&self, result: OperationCellResult) {
        if self.result_tx.borrow().is_none() {
            self.result_tx.send_replace(Some(Arc::new(result)));
        }
    }

    async fn wait(
        &self,
        timeout: Duration,
    ) -> Result<Arc<OperationCellResult>, OperationWaitError> {
        let mut receiver = self.result_tx.subscribe();
        let wait = async {
            loop {
                if let Some(result) = receiver.borrow().as_ref().cloned() {
                    return Ok(result);
                }
                receiver
                    .changed()
                    .await
                    .map_err(|_| OperationWaitError::TransientFailure)?;
            }
        };
        tokio::time::timeout(timeout, wait)
            .await
            .map_err(|_| OperationWaitError::InProgress)?
    }
}

#[derive(Default)]
struct RegistryInner {
    cells: Mutex<HashMap<OperationKey, Arc<OperationCell>>>,
}

#[derive(Clone, Default)]
pub(crate) struct LifecycleOperationRegistry {
    inner: Arc<RegistryInner>,
}

pub(crate) enum OperationRegistration {
    Owner(OperationOwnerGuard),
    Join(OperationJoinHandle),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum OperationRegistrationError {
    #[error("operation_id was reused with a different lifecycle request")]
    FingerprintConflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum OperationWaitError {
    #[error("operation is still in progress")]
    InProgress,
    #[error("operation owner failed before publishing a durable result")]
    TransientFailure,
}

impl LifecycleOperationRegistry {
    /// Atomic insert-or-get. The registry mutex is released before this method
    /// returns, so callers cannot accidentally nest it with coordinator or DB
    /// work.
    pub fn register(
        &self,
        key: OperationKey,
        fingerprint: &str,
    ) -> Result<OperationRegistration, OperationRegistrationError> {
        let mut cells = self
            .inner
            .cells
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(cell) = cells.get(&key) {
            if cell.fingerprint != fingerprint {
                return Err(OperationRegistrationError::FingerprintConflict);
            }
            return Ok(OperationRegistration::Join(OperationJoinHandle {
                cell: Arc::clone(cell),
            }));
        }
        let cell = OperationCell::new(fingerprint.to_owned());
        cells.insert(key.clone(), Arc::clone(&cell));
        Ok(OperationRegistration::Owner(OperationOwnerGuard {
            registry: Arc::downgrade(&self.inner),
            key,
            cell,
            published: false,
        }))
    }

    #[cfg(test)]
    fn contains(&self, key: &OperationKey) -> bool {
        self.inner
            .cells
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .contains_key(key)
    }
}

pub(crate) struct OperationJoinHandle {
    cell: Arc<OperationCell>,
}

impl OperationJoinHandle {
    pub async fn wait(
        &self,
        timeout: Duration,
    ) -> Result<Arc<OperationCellResult>, OperationWaitError> {
        self.cell.wait(timeout).await
    }
}

/// Conditional-remove RAII owner. Panic/abort publishes a transient result so
/// joiners never wait on a dead cell forever.
pub(crate) struct OperationOwnerGuard {
    registry: Weak<RegistryInner>,
    key: OperationKey,
    cell: Arc<OperationCell>,
    published: bool,
}

impl OperationOwnerGuard {
    pub fn join_handle(&self) -> OperationJoinHandle {
        OperationJoinHandle {
            cell: Arc::clone(&self.cell),
        }
    }

    pub fn publish(mut self, result: OperationCellResult) {
        self.cell.publish(result);
        self.remove_if_current();
        self.published = true;
    }

    fn remove_if_current(&self) {
        let Some(registry) = self.registry.upgrade() else {
            return;
        };
        let mut cells = registry
            .cells
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if cells
            .get(&self.key)
            .is_some_and(|current| Arc::ptr_eq(current, &self.cell))
        {
            cells.remove(&self.key);
        }
    }
}

impl Drop for OperationOwnerGuard {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        self.cell.publish(OperationCellResult::TransientFailure);
        self.remove_if_current();
    }
}

/// One owner task's guard and child-process survival domain.
///
/// The child handle stays inside this value while it is awaited by mutable
/// reference. If the owner task is aborted, Drop transfers the handle and all
/// guards to a detached reaper; guards are not released until the child is
/// quiescent.
pub(crate) struct MutationSupervisor<T: Send + 'static> {
    guards: Vec<Box<dyn Any + Send>>,
    child: Option<JoinHandle<T>>,
}

impl<T: Send + 'static> MutationSupervisor<T> {
    pub fn new() -> Self {
        Self {
            guards: Vec::new(),
            child: None,
        }
    }

    pub fn insert_guard<G: Any + Send>(&mut self, guard: G) {
        self.guards.push(Box::new(guard));
    }

    pub fn take_guard<G: Any + Send>(&mut self) -> Option<G> {
        let index = self.guards.iter().position(|guard| guard.is::<G>())?;
        self.guards
            .swap_remove(index)
            .downcast::<G>()
            .ok()
            .map(|guard| *guard)
    }

    pub fn spawn_child<F>(&mut self, future: F)
    where
        F: Future<Output = T> + Send + 'static,
    {
        assert!(
            self.child.is_none(),
            "mutation supervisor already owns a child"
        );
        self.child = Some(tokio::spawn(future));
    }

    pub fn spawn_blocking_child<F>(&mut self, operation: F)
    where
        F: FnOnce() -> T + Send + 'static,
    {
        assert!(
            self.child.is_none(),
            "mutation supervisor already owns a child"
        );
        self.child = Some(tokio::task::spawn_blocking(operation));
    }

    /// Once a child is spawned, owner code must await this immediately. The
    /// JoinHandle is polled by mutable reference and remains reaper-owned if
    /// the outer future is cancelled during this await.
    pub async fn join_child(&mut self) -> Result<T, JoinError> {
        let result = self
            .child
            .as_mut()
            .expect("mutation supervisor has no child")
            .await;
        self.child.take();
        result
    }
}

impl<T: Send + 'static> Drop for MutationSupervisor<T> {
    fn drop(&mut self) {
        let Some(child) = self.child.take() else {
            return;
        };
        let guards = std::mem::take(&mut self.guards);
        let runtime = tokio::runtime::Handle::try_current()
            .expect("mutation supervisor dropped outside a Tokio runtime");
        runtime.spawn(async move {
            let _ = child.await;
            drop(guards);
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LifecycleFingerprint {
    pub canonical: String,
    pub endpoint_keys: Vec<String>,
}

#[derive(Serialize)]
struct CanonicalLifecycleFingerprint<'a> {
    kind: &'a str,
    thread_id: &'a str,
    endpoint_keys: &'a [String],
}

pub(crate) fn canonical_lifecycle_fingerprint(
    kind: LifecycleOperationKind,
    thread_id: &str,
    endpoint_keys: impl IntoIterator<Item = String>,
) -> Result<LifecycleFingerprint, serde_json::Error> {
    let mut endpoint_keys = endpoint_keys
        .into_iter()
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty())
        .collect::<Vec<_>>();
    endpoint_keys.sort();
    endpoint_keys.dedup();
    let kind = match kind {
        LifecycleOperationKind::Archive => "archive",
        LifecycleOperationKind::Delete => "delete",
    };
    let canonical = serde_json::to_string(&CanonicalLifecycleFingerprint {
        kind,
        thread_id: thread_id.trim(),
        endpoint_keys: &endpoint_keys,
    })?;
    Ok(LifecycleFingerprint {
        canonical,
        endpoint_keys,
    })
}

/// Lifecycle registry plus durable cleanup-outbox worker.
pub(crate) struct LifecycleService {
    pub registry: LifecycleOperationRegistry,
    db: Arc<GaryxDbService>,
    state: OnceLock<Weak<AppState>>,
    wake: Notify,
    worker_started: AtomicBool,
    #[cfg(any(test, feature = "test-seams"))]
    pause_after_initial_lookup: Mutex<Option<Arc<LifecyclePauseState>>>,
    #[cfg(any(test, feature = "test-seams"))]
    panic_owner_once: AtomicBool,
    #[cfg(any(test, feature = "test-seams"))]
    fail_after_provider_clear_once: AtomicBool,
}

impl LifecycleService {
    pub fn new(db: Arc<GaryxDbService>) -> Arc<Self> {
        Arc::new(Self {
            registry: LifecycleOperationRegistry::default(),
            db,
            state: OnceLock::new(),
            wake: Notify::new(),
            worker_started: AtomicBool::new(false),
            #[cfg(any(test, feature = "test-seams"))]
            pause_after_initial_lookup: Mutex::new(None),
            #[cfg(any(test, feature = "test-seams"))]
            panic_owner_once: AtomicBool::new(false),
            #[cfg(any(test, feature = "test-seams"))]
            fail_after_provider_clear_once: AtomicBool::new(false),
        })
    }

    pub fn attach_state(&self, state: Weak<AppState>) {
        let _ = self.state.set(state);
    }

    pub fn wake_outbox(&self) {
        self.wake.notify_one();
    }

    pub fn start_outbox_worker(self: &Arc<Self>) {
        if self
            .worker_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let service = Arc::clone(self);
        tokio::spawn(async move {
            service.run_worker().await;
        });
    }

    #[cfg(any(test, feature = "test-seams"))]
    pub fn pause_after_initial_lookup_once(&self) -> TestLifecyclePause {
        let state = Arc::new(LifecyclePauseState {
            started: AtomicBool::new(false),
            started_notify: Notify::new(),
            released: AtomicBool::new(false),
            release_notify: Notify::new(),
        });
        *self
            .pause_after_initial_lookup
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(Arc::clone(&state));
        TestLifecyclePause { state }
    }

    #[cfg(any(test, feature = "test-seams"))]
    pub async fn pause_after_initial_lookup_if_configured(&self) {
        let pause = self
            .pause_after_initial_lookup
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        let Some(pause) = pause else {
            return;
        };
        pause.started.store(true, Ordering::Release);
        pause.started_notify.notify_waiters();
        loop {
            if pause.released.load(Ordering::Acquire) {
                return;
            }
            let notified = pause.release_notify.notified();
            if pause.released.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }

    #[cfg(any(test, feature = "test-seams"))]
    pub fn panic_owner_once(&self) {
        self.panic_owner_once.store(true, Ordering::Release);
    }

    #[cfg(any(test, feature = "test-seams"))]
    pub fn take_owner_panic(&self) -> bool {
        self.panic_owner_once.swap(false, Ordering::AcqRel)
    }

    async fn run_worker(self: Arc<Self>) {
        loop {
            let mut progressed = false;
            loop {
                match self.process_one_ready_job().await {
                    Ok(true) => progressed = true,
                    Ok(false) => break,
                    Err(error) => {
                        tracing::warn!(error = %error, "cleanup outbox worker failed to poll");
                        break;
                    }
                }
            }
            if self.state.get().and_then(Weak::upgrade).is_none() {
                return;
            }
            if progressed {
                tokio::task::yield_now().await;
                continue;
            }
            tokio::select! {
                _ = self.wake.notified() => {}
                _ = tokio::time::sleep(OUTBOX_IDLE_POLL) => {}
            }
        }
    }

    pub async fn process_one_ready_job(&self) -> Result<bool, String> {
        let db = Arc::clone(&self.db);
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let job = db
            .run_blocking(move |db| db.next_cleanup_outbox_job(&now))
            .await
            .map_err(|error| error.to_string())?;
        let Some(job) = job else {
            return Ok(false);
        };
        match self.execute_step(&job).await {
            Ok(()) => {
                let db = Arc::clone(&self.db);
                let job_id = job.job_id;
                db.run_blocking(move |db| db.mark_cleanup_outbox_done(job_id))
                    .await
                    .map_err(|error| error.to_string())?;
            }
            Err(error) => {
                let next_attempt = job.attempt_count.saturating_add(1);
                let delay = cleanup_backoff(next_attempt);
                let next_attempt_at = (Utc::now()
                    + chrono::Duration::from_std(delay)
                        .unwrap_or_else(|_| chrono::Duration::seconds(60)))
                .to_rfc3339_opts(SecondsFormat::Millis, true);
                let db = Arc::clone(&self.db);
                let job_id = job.job_id;
                db.run_blocking(move |db| db.retry_cleanup_outbox_job(job_id, &next_attempt_at))
                    .await
                    .map_err(|db_error| db_error.to_string())?;
                if next_attempt >= OUTBOX_WARNING_ATTEMPT {
                    tracing::warn!(
                        thread_id = %job.thread_id,
                        step = ?job.step,
                        attempt = next_attempt,
                        error = %error,
                        "cleanup outbox job remains retryable"
                    );
                } else {
                    tracing::debug!(
                        thread_id = %job.thread_id,
                        step = ?job.step,
                        attempt = next_attempt,
                        error = %error,
                        "cleanup outbox job scheduled for retry"
                    );
                }
            }
        }
        Ok(true)
    }

    async fn execute_step(&self, job: &CleanupOutboxJob) -> Result<(), String> {
        let state = self
            .state
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| "gateway state is unavailable".to_owned())?;
        match job.step {
            CleanupOutboxStep::EndpointRuntimeInvalidate => {
                let endpoint_key = required_payload_string(job, "endpoint_key")?;
                let expected_thread_id = required_payload_string(job, "expected_thread_id")?;
                state
                    .threads
                    .router
                    .lock()
                    .await
                    .purge_endpoint_binding_if_owned(&endpoint_key, &expected_thread_id);
                state.invalidate_channel_endpoint_cache().await;
                Ok(())
            }
            CleanupOutboxStep::RuntimeTeardown => {
                let provider_key = optional_payload_string(job, "provider_key");
                match state
                    .integration
                    .bridge
                    .clear_thread_state(&job.thread_id, provider_key.as_deref())
                    .await
                {
                    ClearSessionOutcome::RetryableFailure => {
                        return Err("provider runtime teardown failed".to_owned());
                    }
                    ClearSessionOutcome::Cleared | ClearSessionOutcome::AlreadyAbsent => {}
                }
                #[cfg(any(test, feature = "test-seams"))]
                if self
                    .fail_after_provider_clear_once
                    .swap(false, Ordering::AcqRel)
                {
                    return Err("injected crash after provider clear".to_owned());
                }
                state
                    .integration
                    .bridge
                    .drop_thread_state(&job.thread_id)
                    .await;
                let mut router = state.threads.router.lock().await;
                router.purge_thread_from_indexes(&job.thread_id);
                router.clear_last_delivery(&job.thread_id);
                drop(router);
                state.invalidate_gateway_sync_caches().await;
                Ok(())
            }
            CleanupOutboxStep::TranscriptRemove => state
                .threads
                .history
                .delete_thread_history(&job.thread_id)
                .await
                .map_err(|error| error.to_string()),
            CleanupOutboxStep::ThreadLogRemove => {
                state.ops.thread_logs.delete_thread(&job.thread_id).await
            }
            CleanupOutboxStep::PromptAttachmentsRemove => state
                .ops
                .prompt_attachments
                .delete_thread_attachments(&job.thread_id)
                .await
                .map_err(|error| error.to_string()),
        }
    }

    #[cfg(any(test, feature = "test-seams"))]
    pub fn fail_after_provider_clear_once(&self) {
        self.fail_after_provider_clear_once
            .store(true, Ordering::Release);
    }
}

fn required_payload_string(job: &CleanupOutboxJob, field: &str) -> Result<String, String> {
    optional_payload_string(job, field).ok_or_else(|| {
        format!(
            "cleanup outbox job {} is missing string payload field '{field}'",
            job.job_id
        )
    })
}

fn optional_payload_string(job: &CleanupOutboxJob, field: &str) -> Option<String> {
    job.payload
        .as_ref()
        .and_then(|payload| payload.get(field))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn cleanup_backoff(attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(6);
    Duration::from_secs((1u64 << shift).min(60))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_bootstrap::AppStateBuilder;
    use crate::application::chat::contracts::IdempotencyScope;
    use crate::endpoint_binding_mutator::{DeleteBindingPreflight, SqlEndpointBindingMutator};
    use crate::garyx_db::{LifecycleMutationInput, LifecycleTransactionResult};
    use crate::prompt_attachment_lifecycle::PromptAttachmentUpload;
    use async_trait::async_trait;
    use garyx_bridge::provider_trait::StreamCallback;
    use garyx_bridge::{BridgeError, ProviderRuntime};
    use garyx_models::config::GaryxConfig;
    use garyx_models::provider::{
        AgentRunRequest, PromptAttachment, PromptAttachmentKind, ProviderRunOptions,
        ProviderRunResult, ProviderType,
    };
    use garyx_router::{
        AdmittedRun, ChannelBinding, EndpointBindingMutationError, EndpointBindingMutator,
        InMemoryThreadStore, RunAdmissionError, ThreadStore,
    };
    use std::collections::BTreeSet;
    use std::sync::atomic::AtomicUsize;

    fn key() -> OperationKey {
        OperationKey {
            store_incarnation: "00000000-0000-4000-8000-000000000001".to_owned(),
            operation_id: "00000000-0000-4000-8000-000000000002".to_owned(),
        }
    }

    #[tokio::test]
    async fn registry_joins_same_fingerprint_and_rejects_mismatch() {
        let registry = LifecycleOperationRegistry::default();
        let OperationRegistration::Owner(owner) =
            registry.register(key(), "fingerprint-a").unwrap()
        else {
            panic!("first registration must own");
        };
        let OperationRegistration::Join(joiner) =
            registry.register(key(), "fingerprint-a").unwrap()
        else {
            panic!("second registration must join");
        };
        assert!(matches!(
            registry.register(key(), "fingerprint-b"),
            Err(OperationRegistrationError::FingerprintConflict)
        ));
        drop(owner);
        assert!(matches!(
            joiner.wait(Duration::from_secs(1)).await.as_deref(),
            Ok(OperationCellResult::TransientFailure)
        ));
        assert!(!registry.contains(&key()));
    }

    struct DropProbe {
        drops: Arc<AtomicUsize>,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn supervisor_abort_reaps_child_before_releasing_guards() {
        let drops = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let drops_in_task = Arc::clone(&drops);
        let started_in_task = Arc::clone(&started);
        let release_in_task = Arc::clone(&release);
        let owner = tokio::spawn(async move {
            let mut supervisor = MutationSupervisor::<()>::new();
            supervisor.insert_guard(DropProbe {
                drops: drops_in_task,
            });
            supervisor.spawn_child(async move {
                started_in_task.notify_one();
                release_in_task.notified().await;
            });
            supervisor.join_child().await.unwrap();
        });
        started.notified().await;
        owner.abort();
        let _ = owner.await;
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(1), async {
            while drops.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn aborted_supervisor_keeps_operation_cell_and_binding_freeze_until_child_quiesces() {
        let thread_id = "thread::supervisor-real-guards";
        let db = Arc::new(GaryxDbService::memory().unwrap());
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        store
            .set(thread_id, serde_json::json!({"thread_id": thread_id}))
            .await
            .unwrap();
        let mutator = Arc::new(SqlEndpointBindingMutator::new(store, db));
        let DeleteBindingPreflight::Frozen { guard: freeze, .. } = mutator
            .preflight_and_freeze(thread_id, || Arc::new(GaryxConfig::default()))
            .await
            .unwrap()
        else {
            panic!("test preflight must install a freeze");
        };
        let registry = LifecycleOperationRegistry::default();
        let OperationRegistration::Owner(operation_owner) =
            registry.register(key(), "fingerprint-a").unwrap()
        else {
            panic!("test operation must own its cell");
        };

        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let started_in_task = Arc::clone(&started);
        let release_in_task = Arc::clone(&release);
        let owner_task = tokio::spawn(async move {
            let mut supervisor = MutationSupervisor::<()>::new();
            supervisor.insert_guard(operation_owner);
            supervisor.insert_guard(freeze);
            supervisor.spawn_child(async move {
                started_in_task.notify_one();
                release_in_task.notified().await;
            });
            supervisor.join_child().await.unwrap();
        });
        started.notified().await;
        owner_task.abort();
        assert!(owner_task.await.unwrap_err().is_cancelled());

        let blocked_bind = mutator
            .bind_endpoint(
                thread_id,
                ChannelBinding {
                    channel: "telegram".to_owned(),
                    account_id: "main".to_owned(),
                    binding_key: "u1".to_owned(),
                    chat_id: "u1".to_owned(),
                    delivery_target_type: "chat_id".to_owned(),
                    delivery_target_id: "u1".to_owned(),
                    display_label: "Test User".to_owned(),
                    last_inbound_at: None,
                    last_delivery_at: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            blocked_bind,
            EndpointBindingMutationError::ThreadLifecycleInProgress(ref id) if id == thread_id
        ));
        let OperationRegistration::Join(joiner) =
            registry.register(key(), "fingerprint-a").unwrap()
        else {
            panic!("old child must keep the operation cell occupied");
        };
        assert!(matches!(
            joiner.wait(Duration::from_millis(20)).await,
            Err(OperationWaitError::InProgress)
        ));

        release.notify_one();
        assert!(matches!(
            joiner.wait(Duration::from_secs(1)).await.as_deref(),
            Ok(OperationCellResult::TransientFailure)
        ));
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match registry.register(key(), "fingerprint-a").unwrap() {
                    OperationRegistration::Owner(owner) => {
                        drop(owner);
                        break;
                    }
                    OperationRegistration::Join(_) => tokio::task::yield_now().await,
                }
            }
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let binding = ChannelBinding {
                    channel: "telegram".to_owned(),
                    account_id: "main".to_owned(),
                    binding_key: "u2".to_owned(),
                    chat_id: "u2".to_owned(),
                    delivery_target_type: "chat_id".to_owned(),
                    delivery_target_id: "u2".to_owned(),
                    display_label: "Test User".to_owned(),
                    last_inbound_at: None,
                    last_delivery_at: None,
                };
                match mutator.bind_endpoint(thread_id, binding).await {
                    Ok(_) => break,
                    Err(EndpointBindingMutationError::ThreadLifecycleInProgress(_)) => {
                        tokio::task::yield_now().await;
                    }
                    Err(error) => panic!("unexpected bind error after reaper: {error}"),
                }
            }
        })
        .await
        .expect("freeze must release only after child quiescence");
    }

    #[tokio::test]
    async fn reaper_applies_child_commit_witness_before_releasing_reservation() {
        let thread_id = "thread::supervisor-commit";
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        store
            .set(thread_id, serde_json::json!({"thread_id": thread_id}))
            .await
            .unwrap();
        let coordinator = store.run_coordinator();
        let reservation = coordinator
            .reserve_delete(store.as_ref(), thread_id)
            .await
            .unwrap();
        let witness = reservation.commit_witness();
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let started_in_task = Arc::clone(&started);
        let release_in_task = Arc::clone(&release);
        let owner = tokio::spawn(async move {
            let mut supervisor = MutationSupervisor::<()>::new();
            supervisor.insert_guard(reservation);
            supervisor.spawn_child(async move {
                witness.mark_committed(Some(garyx_router::ThreadTerminalState::Deleted));
                started_in_task.notify_one();
                release_in_task.notified().await;
            });
            supervisor.join_child().await.unwrap();
        });
        started.notified().await;
        owner.abort();
        let _ = owner.await;
        assert!(
            coordinator.mutation_in_progress(thread_id),
            "reservation was released while the committed child remained live"
        );
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(1), async {
            while coordinator.mutation_in_progress(thread_id) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let request = AgentRunRequest::new(
            thread_id,
            "hello",
            "run::supervisor-commit",
            "api",
            "main",
            HashMap::new(),
        );
        assert!(matches!(
            AdmittedRun::thread_bound(store, request).await,
            Err(RunAdmissionError::Archived(_))
        ));
    }

    #[test]
    fn lifecycle_fingerprint_is_sorted_deduplicated_complete_json() {
        let fingerprint = canonical_lifecycle_fingerprint(
            LifecycleOperationKind::Archive,
            "thread::one",
            ["z".to_owned(), " a ".to_owned(), "z".to_owned()],
        )
        .unwrap();
        assert_eq!(fingerprint.endpoint_keys, vec!["a", "z"]);
        assert_eq!(
            fingerprint.canonical,
            r#"{"kind":"archive","thread_id":"thread::one","endpoint_keys":["a","z"]}"#
        );
    }

    #[test]
    fn cleanup_backoff_is_bounded_and_monotonic() {
        assert_eq!(cleanup_backoff(1), Duration::from_secs(1));
        assert_eq!(cleanup_backoff(2), Duration::from_secs(2));
        assert_eq!(cleanup_backoff(7), Duration::from_secs(60));
        assert_eq!(cleanup_backoff(100), Duration::from_secs(60));
    }

    struct CleanupProvider {
        retryable_failure: AtomicBool,
        present: AtomicBool,
        calls: AtomicUsize,
    }

    impl CleanupProvider {
        fn new(retryable_failure: bool) -> Self {
            Self {
                retryable_failure: AtomicBool::new(retryable_failure),
                present: AtomicBool::new(true),
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl ProviderRuntime for CleanupProvider {
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
            _options: &ProviderRunOptions,
            _on_chunk: StreamCallback,
        ) -> Result<ProviderRunResult, BridgeError> {
            Err(BridgeError::Internal("not used by cleanup test".to_owned()))
        }

        async fn get_or_create_session(&self, thread_id: &str) -> Result<String, BridgeError> {
            Ok(format!("session-{thread_id}"))
        }

        async fn clear_session(&self, _thread_id: &str) -> ClearSessionOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.retryable_failure.load(Ordering::Acquire) {
                return ClearSessionOutcome::RetryableFailure;
            }
            if self.present.swap(false, Ordering::AcqRel) {
                ClearSessionOutcome::Cleared
            } else {
                ClearSessionOutcome::AlreadyAbsent
            }
        }
    }

    async fn state_with_runtime_cleanup(
        provider: Arc<CleanupProvider>,
        operation_kind: LifecycleOperationKind,
    ) -> (
        Arc<AppState>,
        Arc<GaryxDbService>,
        String,
        String,
        String,
        tempfile::TempDir,
    ) {
        let db = Arc::new(GaryxDbService::memory().expect("db opens"));
        let bridge = Arc::new(garyx_bridge::MultiProviderBridge::new());
        bridge.register_provider("provider-one", provider).await;
        let attachment_data = tempfile::tempdir().unwrap();
        let mut config = GaryxConfig::default();
        config.sessions.data_dir = Some(attachment_data.path().join("data").display().to_string());
        let state = AppStateBuilder::new(config)
            .with_garyx_db(db.clone())
            .with_bridge(bridge.clone())
            .build();
        let thread_id = "thread::outbox-runtime".to_owned();
        state
            .threads
            .thread_store
            .set(
                &thread_id,
                serde_json::json!({
                    "thread_id": thread_id,
                    "provider_key": "provider-one"
                }),
            )
            .await
            .unwrap();
        bridge.set_thread_affinity(&thread_id, "provider-one").await;
        bridge
            .set_thread_workspace_binding(&thread_id, Some("/tmp/test-workspace".to_owned()))
            .await;
        let scope = IdempotencyScope {
            identity: "outbox-attachment".to_owned(),
            epoch: 1,
        };
        let uploaded = state
            .ops
            .prompt_attachments
            .upload(
                Some(&scope),
                vec![PromptAttachmentUpload {
                    kind: PromptAttachmentKind::Image,
                    name: "outbox-photo.jpg".to_owned(),
                    media_type: "image/jpeg".to_owned(),
                    bytes: vec![0xff, 0xd8, 0xff, 0xd9],
                }],
            )
            .await
            .unwrap()
            .pop()
            .unwrap();
        let mut attachments = vec![PromptAttachment {
            attachment_id: Some(uploaded.attachment_id.clone()),
            kind: uploaded.kind.clone(),
            path: uploaded.path.clone(),
            name: uploaded.name,
            media_type: uploaded.media_type,
        }];
        let claims = state
            .ops
            .prompt_attachments
            .prepare_claims((&scope.identity, scope.epoch), &mut attachments)
            .await
            .unwrap();
        state
            .ops
            .prompt_attachments
            .claim_standalone(
                (&scope.identity, scope.epoch),
                &thread_id,
                crate::garyx_db::DispatchAdmissionKind::ChatStart,
                Some("outbox-intent"),
                Some("outbox-run"),
                "outbox-run",
                &claims,
            )
            .await
            .unwrap();
        let attachment_path = attachments[0].path.clone();
        let attachment_id = attachments[0].attachment_id.clone().unwrap();
        let incarnation = db.store_incarnation_id().unwrap();
        let operation_label = match operation_kind {
            LifecycleOperationKind::Archive => "archive",
            LifecycleOperationKind::Delete => "delete",
        };
        let result = db
            .execute_lifecycle_mutation(LifecycleMutationInput {
                expected_store_incarnation: incarnation,
                operation_id: format!("operation-runtime-cleanup-{operation_label}"),
                kind: operation_kind,
                thread_id: thread_id.clone(),
                fingerprint: format!("runtime-cleanup-{operation_label}-fingerprint"),
                endpoint_keys: Vec::new(),
                enabled_channel_accounts: BTreeSet::new(),
            })
            .unwrap();
        assert!(matches!(
            result,
            LifecycleTransactionResult::Completed { .. }
        ));
        (
            state,
            db,
            thread_id,
            attachment_path,
            attachment_id,
            attachment_data,
        )
    }

    #[tokio::test]
    async fn retryable_provider_cleanup_retains_affinity_and_pending_job() {
        let provider = Arc::new(CleanupProvider::new(true));
        let (state, db, thread_id, attachment_path, _, _attachment_data) =
            state_with_runtime_cleanup(provider.clone(), LifecycleOperationKind::Delete).await;
        assert!(state.ops.lifecycle.process_one_ready_job().await.unwrap());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            state
                .integration
                .bridge
                .thread_affinity_for(&thread_id)
                .await
                .as_deref(),
            Some("provider-one")
        );
        assert_eq!(db.pending_cleanup_outbox_count().unwrap(), 4);
        assert!(std::path::Path::new(&attachment_path).exists());
    }

    #[tokio::test]
    async fn replay_after_provider_clear_before_local_drop_converges_from_already_absent() {
        let provider = Arc::new(CleanupProvider::new(false));
        let (state, db, thread_id, attachment_path, _, _attachment_data) =
            state_with_runtime_cleanup(provider.clone(), LifecycleOperationKind::Delete).await;
        state.ops.lifecycle.fail_after_provider_clear_once();
        assert!(state.ops.lifecycle.process_one_ready_job().await.unwrap());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert!(
            state
                .integration
                .bridge
                .thread_affinity_for(&thread_id)
                .await
                .is_some(),
            "local affinity was dropped before the durable retry point"
        );
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        assert!(state.ops.lifecycle.process_one_ready_job().await.unwrap());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
        assert!(
            state
                .integration
                .bridge
                .thread_affinity_for(&thread_id)
                .await
                .is_none()
        );
        assert_eq!(db.pending_cleanup_outbox_count().unwrap(), 3);
        assert!(std::path::Path::new(&attachment_path).exists());
    }

    #[tokio::test]
    async fn archive_outbox_retains_thread_owned_attachments() {
        let provider = Arc::new(CleanupProvider::new(false));
        let (state, db, _, attachment_path, attachment_id, _attachment_data) =
            state_with_runtime_cleanup(provider, LifecycleOperationKind::Archive).await;
        assert_eq!(db.pending_cleanup_outbox_count().unwrap(), 3);

        while state.ops.lifecycle.process_one_ready_job().await.unwrap() {}

        assert_eq!(db.pending_cleanup_outbox_count().unwrap(), 0);
        assert!(std::path::Path::new(&attachment_path).exists());
        assert!(
            db.prompt_attachment_by_id(&attachment_id)
                .unwrap()
                .is_some(),
            "archiving a thread must retain its durable conversation attachments"
        );
    }

    #[tokio::test]
    async fn pending_outbox_survives_state_restart_and_boot_worker_drains_it() {
        let original_provider = Arc::new(CleanupProvider::new(false));
        let (original_state, db, thread_id, attachment_path, attachment_id, attachment_data) =
            state_with_runtime_cleanup(original_provider, LifecycleOperationKind::Delete).await;
        assert_eq!(db.pending_cleanup_outbox_count().unwrap(), 4);
        drop(original_state);

        // A fresh lifecycle service/bridge represents the next gateway boot;
        // only the SQLite outbox crosses this boundary.
        let restarted_provider = Arc::new(CleanupProvider::new(false));
        let restarted_bridge = Arc::new(garyx_bridge::MultiProviderBridge::new());
        restarted_bridge
            .register_provider("provider-one", restarted_provider.clone())
            .await;
        restarted_bridge
            .set_thread_affinity(&thread_id, "provider-one")
            .await;
        restarted_bridge
            .set_thread_workspace_binding(&thread_id, Some("/tmp/stale-workspace".to_owned()))
            .await;
        let mut restarted_config = GaryxConfig::default();
        restarted_config.sessions.data_dir =
            Some(attachment_data.path().join("data").display().to_string());
        let restarted = AppStateBuilder::new(restarted_config)
            .with_garyx_db(db.clone())
            .with_bridge(restarted_bridge.clone())
            .build();
        restarted.ops.lifecycle.start_outbox_worker();

        tokio::time::timeout(Duration::from_secs(2), async {
            while db.pending_cleanup_outbox_count().unwrap() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("boot worker must drain every persisted pending job");
        assert_eq!(restarted_provider.calls.load(Ordering::SeqCst), 1);
        assert!(
            restarted_bridge
                .thread_affinity_for(&thread_id)
                .await
                .is_none()
        );
        assert!(!std::path::Path::new(&attachment_path).exists());
        assert!(
            db.prompt_attachment_by_id(&attachment_id)
                .unwrap()
                .is_none()
        );
    }
}
