use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use garyx_models::provider::{AgentRunRequest, ProviderType};
use garyx_models::strip_server_owned_agent_metadata;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::{ThreadStore, is_thread_key};

#[derive(Debug, thiserror::Error)]
pub enum RunAdmissionError {
    #[error("thread not found: {0}")]
    NotFound(String),
    #[error("thread is archived: {0}")]
    Archived(String),
    #[error("thread is being archived or deleted: {0}")]
    Stale(String),
    #[error("invalid canonical thread id: {0}")]
    InvalidThreadId(String),
    #[error("provider tool runtime id must start with tool::: {0}")]
    InvalidProviderToolId(String),
    #[error("thread store backend failed: {0}")]
    Store(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThreadMutationState {
    Live,
    Archiving,
    Archived,
    Deleting,
    Deleted,
}

struct LeaseEntry {
    valid: Arc<AtomicBool>,
    active: bool,
}

struct ThreadCoordinationEntry {
    state: ThreadMutationState,
    next_lease_id: u64,
    leases: HashMap<u64, LeaseEntry>,
}

impl Default for ThreadCoordinationEntry {
    fn default() -> Self {
        Self {
            state: ThreadMutationState::Live,
            next_lease_id: 1,
            leases: HashMap::new(),
        }
    }
}

#[async_trait]
pub trait ThreadRunAborter: Send + Sync {
    /// Abort every provider task for `thread_id` and do not return until all
    /// task futures have reached a terminal state.
    async fn abort_and_drain_thread(&self, thread_id: &str) -> Result<(), String>;
}

/// Store-owned linearization domain for run admission and destructive thread
/// mutations. All guards release synchronously in `Drop`, so cancellation and
/// panic cannot strand a lease.
pub struct ThreadRunCoordinator {
    entries: Mutex<HashMap<String, ThreadCoordinationEntry>>,
    aborter: Mutex<Option<Arc<dyn ThreadRunAborter>>>,
    changed: Notify,
}

impl Default for ThreadRunCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl ThreadRunCoordinator {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            aborter: Mutex::new(None),
            changed: Notify::new(),
        }
    }

    pub fn shared_fallback() -> Arc<Self> {
        static COORDINATOR: OnceLock<Arc<ThreadRunCoordinator>> = OnceLock::new();
        COORDINATOR
            .get_or_init(|| Arc::new(ThreadRunCoordinator::new()))
            .clone()
    }

    pub fn set_aborter(&self, aborter: Arc<dyn ThreadRunAborter>) {
        *self
            .aborter
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(aborter);
    }

    fn reserve_request(
        self: &Arc<Self>,
        thread_id: &str,
    ) -> Result<ThreadRunLease, RunAdmissionError> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let entry = entries.entry(thread_id.to_owned()).or_default();
        match entry.state {
            ThreadMutationState::Live => {}
            ThreadMutationState::Archived => {
                return Err(RunAdmissionError::Archived(thread_id.to_owned()));
            }
            ThreadMutationState::Archiving
            | ThreadMutationState::Deleting
            | ThreadMutationState::Deleted => {
                return Err(RunAdmissionError::Stale(thread_id.to_owned()));
            }
        }
        let lease_id = entry.next_lease_id;
        entry.next_lease_id = entry.next_lease_id.saturating_add(1);
        let valid = Arc::new(AtomicBool::new(true));
        entry.leases.insert(
            lease_id,
            LeaseEntry {
                valid: valid.clone(),
                active: false,
            },
        );
        Ok(ThreadRunLease {
            coordinator: Arc::clone(self),
            thread_id: thread_id.to_owned(),
            lease_id,
            valid,
            released: false,
        })
    }

    pub fn lease_count(&self, thread_id: &str) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(thread_id)
            .map(|entry| entry.leases.len())
            .unwrap_or(0)
    }

    pub fn has_active_lease(&self, thread_id: &str) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(thread_id)
            .is_some_and(|entry| entry.leases.values().any(|lease| lease.active))
    }

    pub fn mutation_in_progress(&self, thread_id: &str) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(thread_id)
            .is_some_and(|entry| {
                matches!(
                    entry.state,
                    ThreadMutationState::Archiving | ThreadMutationState::Deleting
                )
            })
    }

    /// Reserve archive before any endpoint side effect and spawn an owned
    /// mutation future. Dropping the caller's JoinHandle detaches the task;
    /// the reservation remains until the operation has definitely completed.
    pub fn start_archive<F, T, E>(
        self: &Arc<Self>,
        thread_id: String,
        operation: F,
    ) -> Result<JoinHandle<Result<T, E>>, ArchiveReservationError>
    where
        F: Future<Output = Result<T, E>> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
    {
        {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let entry = entries.entry(thread_id.clone()).or_default();
            if entry.state != ThreadMutationState::Live {
                return Err(ArchiveReservationError::Unavailable);
            }
            if !entry.leases.is_empty() {
                return Err(ArchiveReservationError::ActiveLease);
            }
            entry.state = ThreadMutationState::Archiving;
        }

        let coordinator = Arc::clone(self);
        Ok(tokio::spawn(async move {
            let mut reservation =
                MutationReservation::new(coordinator, thread_id, ThreadMutationState::Archiving);
            match operation.await {
                Ok(value) => {
                    reservation.complete(ThreadMutationState::Archived);
                    Ok(value)
                }
                Err(error) => Err(error),
            }
        }))
    }

    /// Enter deleting, invalidate every request/active token, abort and drain
    /// provider tasks, wait for synchronous guard drops, then run the owned
    /// storage mutation. Every failure restores `live` after it is known that
    /// the record remains.
    pub fn start_delete<F, T, E>(
        self: &Arc<Self>,
        thread_id: String,
        operation: F,
    ) -> JoinHandle<Result<T, CoordinatedDeleteError<E>>>
    where
        F: Future<Output = Result<T, E>> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
    {
        let unavailable = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let entry = entries.entry(thread_id.clone()).or_default();
            match entry.state {
                ThreadMutationState::Live
                | ThreadMutationState::Archived
                | ThreadMutationState::Deleted => {
                    entry.state = ThreadMutationState::Deleting;
                    for lease in entry.leases.values() {
                        lease.valid.store(false, Ordering::Release);
                    }
                    false
                }
                ThreadMutationState::Archiving | ThreadMutationState::Deleting => true,
            }
        };
        if unavailable {
            return tokio::spawn(async { Err(CoordinatedDeleteError::Unavailable) });
        }
        self.changed.notify_waiters();

        let coordinator = Arc::clone(self);
        tokio::spawn(async move {
            let mut reservation = MutationReservation::new(
                Arc::clone(&coordinator),
                thread_id.clone(),
                ThreadMutationState::Deleting,
            );
            let aborter = coordinator
                .aborter
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .as_ref()
                .cloned();
            if let Some(aborter) = aborter {
                aborter
                    .abort_and_drain_thread(&thread_id)
                    .await
                    .map_err(CoordinatedDeleteError::Abort)?;
            }
            coordinator.wait_for_no_leases(&thread_id).await;
            match operation.await {
                Ok(value) => {
                    reservation.complete(ThreadMutationState::Deleted);
                    Ok(value)
                }
                Err(error) => Err(CoordinatedDeleteError::Operation(error)),
            }
        })
    }

    async fn wait_for_no_leases(&self, thread_id: &str) {
        loop {
            let notified = self.changed.notified();
            if self.lease_count(thread_id) == 0 {
                return;
            }
            notified.await;
        }
    }

    fn release_lease(&self, thread_id: &str, lease_id: u64) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(entry) = entries.get_mut(thread_id) {
            entry.leases.remove(&lease_id);
            if entry.state == ThreadMutationState::Live && entry.leases.is_empty() {
                entries.remove(thread_id);
            }
        }
        drop(entries);
        self.changed.notify_waiters();
    }

    /// A successful record write after a completed delete recreates the key
    /// as a new live record. Writes racing an in-progress mutation never
    /// cancel that mutation's reservation.
    pub fn record_written(&self, thread_id: &str) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if entries
            .get(thread_id)
            .is_some_and(|entry| entry.state == ThreadMutationState::Deleted)
        {
            entries.remove(thread_id);
        }
        drop(entries);
        self.changed.notify_waiters();
    }

    fn finish_mutation(
        &self,
        thread_id: &str,
        expected: ThreadMutationState,
        terminal: Option<ThreadMutationState>,
    ) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(entry) = entries.get_mut(thread_id) else {
            return;
        };
        if entry.state != expected {
            return;
        }
        if let Some(terminal) = terminal {
            entry.state = terminal;
        } else if entry.leases.is_empty() {
            entries.remove(thread_id);
        } else {
            entry.state = ThreadMutationState::Live;
        }
        drop(entries);
        self.changed.notify_waiters();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ArchiveReservationError {
    #[error("thread has an admitted or active run")]
    ActiveLease,
    #[error("thread is already being archived or deleted")]
    Unavailable,
}

#[derive(Debug, thiserror::Error)]
pub enum CoordinatedDeleteError<E> {
    #[error("thread is already being archived or deleted")]
    Unavailable,
    #[error("failed to abort and drain thread runs: {0}")]
    Abort(String),
    #[error("thread delete failed: {0}")]
    Operation(E),
}

struct MutationReservation {
    coordinator: Arc<ThreadRunCoordinator>,
    thread_id: String,
    expected: ThreadMutationState,
    completed: bool,
}

impl MutationReservation {
    fn new(
        coordinator: Arc<ThreadRunCoordinator>,
        thread_id: String,
        expected: ThreadMutationState,
    ) -> Self {
        Self {
            coordinator,
            thread_id,
            expected,
            completed: false,
        }
    }

    fn complete(&mut self, terminal: ThreadMutationState) {
        self.coordinator
            .finish_mutation(&self.thread_id, self.expected, Some(terminal));
        self.completed = true;
    }
}

impl Drop for MutationReservation {
    fn drop(&mut self) {
        if !self.completed {
            self.coordinator
                .finish_mutation(&self.thread_id, self.expected, None);
        }
    }
}

/// A per-request lease. It is intentionally non-Clone and releases from Drop.
pub struct ThreadRunLease {
    coordinator: Arc<ThreadRunCoordinator>,
    thread_id: String,
    lease_id: u64,
    valid: Arc<AtomicBool>,
    released: bool,
}

impl ThreadRunLease {
    pub fn ensure_valid(&self) -> Result<(), RunAdmissionError> {
        if self.valid.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(RunAdmissionError::Stale(self.thread_id.clone()))
        }
    }

    pub fn promote_to_active(&mut self) -> Result<(), RunAdmissionError> {
        self.ensure_valid()?;
        let mut entries = self
            .coordinator
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let entry = entries
            .get_mut(&self.thread_id)
            .ok_or_else(|| RunAdmissionError::Stale(self.thread_id.clone()))?;
        if entry.state != ThreadMutationState::Live {
            return Err(RunAdmissionError::Stale(self.thread_id.clone()));
        }
        let lease = entry
            .leases
            .get_mut(&self.lease_id)
            .ok_or_else(|| RunAdmissionError::Stale(self.thread_id.clone()))?;
        if !lease.valid.load(Ordering::Acquire) {
            return Err(RunAdmissionError::Stale(self.thread_id.clone()));
        }
        lease.active = true;
        Ok(())
    }
}

impl Drop for ThreadRunLease {
    fn drop(&mut self) {
        if !self.released {
            self.released = true;
            self.coordinator
                .release_lease(&self.thread_id, self.lease_id);
        }
    }
}

enum AdmittedRunInner {
    ThreadBound {
        request: AgentRunRequest,
        lease: ThreadRunLease,
    },
    ProviderTool {
        request: AgentRunRequest,
    },
}

/// The only production value accepted by an [`crate::AgentDispatcher`]. Its
/// representation is private and it is deliberately non-Clone.
pub struct AdmittedRun {
    inner: AdmittedRunInner,
}

impl AdmittedRun {
    pub fn thread_id(&self) -> &str {
        match &self.inner {
            AdmittedRunInner::ThreadBound { request, .. }
            | AdmittedRunInner::ProviderTool { request } => &request.thread_id,
        }
    }

    /// Point-read the backing store and acquire a request lease. Callers cannot
    /// supply a pre-read record as proof.
    pub async fn thread_bound(
        store: Arc<dyn ThreadStore>,
        request: AgentRunRequest,
    ) -> Result<Self, RunAdmissionError> {
        if !is_thread_key(&request.thread_id) {
            return Err(RunAdmissionError::InvalidThreadId(request.thread_id));
        }
        let coordinator = store.run_coordinator();
        let lease = coordinator.reserve_request(&request.thread_id)?;
        let exists = store
            .get(&request.thread_id)
            .await
            .map_err(|error| RunAdmissionError::Store(error.to_string()))?
            .is_some();
        if !exists {
            return Err(RunAdmissionError::NotFound(request.thread_id));
        }
        if store
            .is_archived(&request.thread_id)
            .await
            .map_err(|error| RunAdmissionError::Store(error.to_string()))?
        {
            return Err(RunAdmissionError::Archived(request.thread_id));
        }
        lease.ensure_valid()?;
        Ok(Self {
            inner: AdmittedRunInner::ThreadBound { request, lease },
        })
    }

    /// Admit an infrastructure provider tool. Only `tool::*` ids are valid;
    /// agent-binding metadata is stripped and the provider comes exclusively
    /// from this sealed constructor argument.
    pub fn provider_tool(
        mut request: AgentRunRequest,
        provider: ProviderType,
    ) -> Result<Self, RunAdmissionError> {
        if !request.thread_id.starts_with("tool::") {
            return Err(RunAdmissionError::InvalidProviderToolId(request.thread_id));
        }
        strip_server_owned_agent_metadata(&mut request.metadata);
        request.requested_provider = Some(provider);
        Ok(Self {
            inner: AdmittedRunInner::ProviderTool { request },
        })
    }

    #[doc(hidden)]
    pub fn into_dispatch_parts(self) -> (AgentRunRequest, Option<ThreadRunLease>) {
        match self.inner {
            AdmittedRunInner::ThreadBound { request, lease } => (request, Some(lease)),
            AdmittedRunInner::ProviderTool { request } => (request, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use garyx_models::provider::ProviderType;
    use serde_json::json;
    use std::sync::atomic::AtomicBool;

    fn request(thread_id: &str) -> AgentRunRequest {
        AgentRunRequest::new(thread_id, "hello", "run::1", "api", "main", HashMap::new())
    }

    #[tokio::test]
    async fn provider_tool_strips_reserved_identity_and_rejects_thread_id() {
        let mut raw_request = request("tool::image::1");
        raw_request
            .metadata
            .insert("agent_id".to_owned(), json!("codex"));
        raw_request
            .metadata
            .insert("model".to_owned(), json!("keep"));
        let admitted =
            AdmittedRun::provider_tool(raw_request, ProviderType::CodexAppServer).unwrap();
        let (tool_request, lease) = admitted.into_dispatch_parts();
        assert!(lease.is_none());
        assert!(!tool_request.metadata.contains_key("agent_id"));
        assert_eq!(tool_request.metadata.get("model"), Some(&json!("keep")));
        assert_eq!(
            tool_request.requested_provider,
            Some(ProviderType::CodexAppServer)
        );

        assert!(matches!(
            AdmittedRun::provider_tool(request("thread::1"), ProviderType::ClaudeCode),
            Err(RunAdmissionError::InvalidProviderToolId(_))
        ));
        assert!(matches!(
            AdmittedRun::provider_tool(request("cron::1"), ProviderType::ClaudeCode),
            Err(RunAdmissionError::InvalidProviderToolId(_))
        ));
    }

    async fn store_with_thread(thread_id: &str) -> Arc<dyn ThreadStore> {
        let store: Arc<dyn ThreadStore> = Arc::new(crate::InMemoryThreadStore::new());
        store.set(thread_id, json!({})).await.unwrap();
        store
    }

    #[tokio::test]
    async fn thread_bound_admission_requires_a_real_record_and_drop_releases_lease() {
        let store: Arc<dyn ThreadStore> = Arc::new(crate::InMemoryThreadStore::new());
        assert!(matches!(
            AdmittedRun::thread_bound(store.clone(), request("thread::missing")).await,
            Err(RunAdmissionError::NotFound(_))
        ));

        store.set("thread::present", json!({})).await.unwrap();
        let admitted = AdmittedRun::thread_bound(store.clone(), request("thread::present"))
            .await
            .unwrap();
        assert_eq!(store.run_coordinator().lease_count("thread::present"), 1);
        drop(admitted);
        assert_eq!(store.run_coordinator().lease_count("thread::present"), 0);
    }

    #[tokio::test]
    async fn archive_with_a_lease_conflicts_before_running_any_side_effect() {
        let thread_id = "thread::archive-conflict";
        let store = store_with_thread(thread_id).await;
        let admitted = AdmittedRun::thread_bound(store.clone(), request(thread_id))
            .await
            .unwrap();
        let side_effect = Arc::new(AtomicBool::new(false));
        let effect = side_effect.clone();
        let result = store
            .run_coordinator()
            .start_archive(thread_id.to_owned(), async move {
                effect.store(true, Ordering::Release);
                Ok::<_, ()>(())
            });
        assert!(matches!(result, Err(ArchiveReservationError::ActiveLease)));
        assert!(!side_effect.load(Ordering::Acquire));
        drop(admitted);
    }

    #[tokio::test]
    async fn archive_reservation_survives_caller_cancellation_and_blocks_admission() {
        let thread_id = "thread::archive-owned";
        let store = store_with_thread(thread_id).await;
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let started_in_task = started.clone();
        let release_in_task = release.clone();
        let handle = store
            .run_coordinator()
            .start_archive(thread_id.to_owned(), async move {
                started_in_task.notify_one();
                release_in_task.notified().await;
                Ok::<_, ()>(())
            })
            .unwrap();
        started.notified().await;
        drop(handle);

        assert!(matches!(
            AdmittedRun::thread_bound(store.clone(), request(thread_id)).await,
            Err(RunAdmissionError::Stale(_))
        ));
        release.notify_one();
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(matches!(
            AdmittedRun::thread_bound(store, request(thread_id)).await,
            Err(RunAdmissionError::Archived(_))
        ));
    }

    #[tokio::test]
    async fn delete_invalidates_tokens_waits_for_drop_and_is_cancellation_safe() {
        let thread_id = "thread::delete-owned";
        let store = store_with_thread(thread_id).await;
        let admitted = AdmittedRun::thread_bound(store.clone(), request(thread_id))
            .await
            .unwrap();
        let (_request, lease) = admitted.into_dispatch_parts();
        let lease = lease.unwrap();
        let operation_started = Arc::new(AtomicBool::new(false));
        let started = operation_started.clone();
        let handle = store
            .run_coordinator()
            .start_delete(thread_id.to_owned(), async move {
                started.store(true, Ordering::Release);
                Ok::<_, ()>(())
            });
        tokio::task::yield_now().await;
        assert!(matches!(
            lease.ensure_valid(),
            Err(RunAdmissionError::Stale(_))
        ));
        assert!(!operation_started.load(Ordering::Acquire));
        assert!(matches!(
            AdmittedRun::thread_bound(store.clone(), request(thread_id)).await,
            Err(RunAdmissionError::Stale(_))
        ));

        // Dropping the caller-owned handle must not cancel deleting. The
        // coordinator task remains blocked until the lease guard drops.
        drop(handle);
        drop(lease);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(operation_started.load(Ordering::Acquire));
        assert!(matches!(
            AdmittedRun::thread_bound(store, request(thread_id)).await,
            Err(RunAdmissionError::Stale(_))
        ));
    }

    #[tokio::test]
    async fn failed_delete_returns_to_live() {
        let thread_id = "thread::delete-failure";
        let store = store_with_thread(thread_id).await;
        let result = store
            .run_coordinator()
            .start_delete(thread_id.to_owned(), async {
                Err::<(), _>("injected delete failure")
            })
            .await
            .unwrap();
        assert!(matches!(result, Err(CoordinatedDeleteError::Operation(_))));
        AdmittedRun::thread_bound(store, request(thread_id))
            .await
            .expect("failed delete must restore live admission");
    }

    #[tokio::test]
    async fn a_record_recreated_after_delete_gets_a_fresh_live_admission_domain() {
        let thread_id = "thread::delete-recreate";
        let store = store_with_thread(thread_id).await;
        assert!(store.delete(thread_id).await.unwrap());
        assert!(!store.delete(thread_id).await.unwrap());
        store.set(thread_id, json!({})).await.unwrap();
        AdmittedRun::thread_bound(store, request(thread_id))
            .await
            .expect("a newly written record must not inherit the deleted tombstone");
    }

    #[tokio::test]
    async fn delete_cannot_overwrite_an_owned_archive_reservation() {
        let thread_id = "thread::archive-delete-race";
        let store = store_with_thread(thread_id).await;
        let archive_started = Arc::new(Notify::new());
        let archive_release = Arc::new(Notify::new());
        let started = archive_started.clone();
        let release = archive_release.clone();
        let archive = store
            .run_coordinator()
            .start_archive(thread_id.to_owned(), async move {
                started.notify_one();
                release.notified().await;
                Ok::<_, ()>(())
            })
            .unwrap();
        archive_started.notified().await;

        let delete_side_effect = Arc::new(AtomicBool::new(false));
        let effect = delete_side_effect.clone();
        let result = store
            .run_coordinator()
            .start_delete(thread_id.to_owned(), async move {
                effect.store(true, Ordering::Release);
                Ok::<_, ()>(())
            })
            .await
            .unwrap();
        assert!(matches!(result, Err(CoordinatedDeleteError::Unavailable)));
        assert!(!delete_side_effect.load(Ordering::Acquire));
        assert!(matches!(
            AdmittedRun::thread_bound(store.clone(), request(thread_id)).await,
            Err(RunAdmissionError::Stale(_))
        ));

        archive_release.notify_one();
        archive.await.unwrap().unwrap();
        assert!(matches!(
            AdmittedRun::thread_bound(store, request(thread_id)).await,
            Err(RunAdmissionError::Archived(_))
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn archive_blocking_mutation_outlives_cancelled_caller() {
        let thread_id = "thread::archive-blocking";
        let store = store_with_thread(thread_id).await;
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let handle = store
            .run_coordinator()
            .start_archive(thread_id.to_owned(), async move {
                tokio::task::spawn_blocking(move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                })
                .await
                .unwrap();
                Ok::<_, ()>(())
            })
            .unwrap();
        started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        drop(handle);
        assert!(matches!(
            AdmittedRun::thread_bound(store.clone(), request(thread_id)).await,
            Err(RunAdmissionError::Stale(_))
        ));
        release_tx.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if matches!(
                    AdmittedRun::thread_bound(store.clone(), request(thread_id)).await,
                    Err(RunAdmissionError::Archived(_))
                ) {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_blocking_mutation_outlives_cancelled_caller() {
        let thread_id = "thread::delete-blocking";
        let store = store_with_thread(thread_id).await;
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let handle = store
            .run_coordinator()
            .start_delete(thread_id.to_owned(), async move {
                tokio::task::spawn_blocking(move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                })
                .await
                .unwrap();
                Ok::<_, ()>(())
            });
        started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        drop(handle);
        assert!(matches!(
            AdmittedRun::thread_bound(store.clone(), request(thread_id)).await,
            Err(RunAdmissionError::Stale(_))
        ));
        release_tx.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if !store.run_coordinator().mutation_in_progress(thread_id)
                    && matches!(
                        AdmittedRun::thread_bound(store.clone(), request(thread_id)).await,
                        Err(RunAdmissionError::Stale(_))
                    )
                {
                    // `Deleted` intentionally presents as typed stale.
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
