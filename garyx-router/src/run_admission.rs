use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use garyx_models::provider::{AgentRunRequest, ProviderType};
use garyx_models::strip_server_owned_agent_metadata;
use tokio::sync::Notify;

use crate::{ThreadStore, ThreadTerminalState, is_thread_key};

#[derive(Debug, thiserror::Error)]
pub enum RunAdmissionError {
    #[error("thread not found: {0}")]
    NotFound(String),
    #[error("thread is terminal: {0}")]
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
enum StableState {
    Live,
    Archived,
    Deleted,
}

impl StableState {
    fn from_terminal(terminal: Option<ThreadTerminalState>) -> Self {
        match terminal {
            None => Self::Live,
            Some(ThreadTerminalState::Archived) => Self::Archived,
            Some(ThreadTerminalState::Deleted) => Self::Deleted,
        }
    }

    fn terminal(self) -> Option<ThreadTerminalState> {
        match self {
            Self::Live => None,
            Self::Archived => Some(ThreadTerminalState::Archived),
            Self::Deleted => Some(ThreadTerminalState::Deleted),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnedStateKind {
    Deciding,
    Archiving,
    Deleting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThreadCoordinationState {
    Uncalibrated,
    Live,
    Deciding { owner_token: u64 },
    Archiving { owner_token: u64 },
    Archived,
    Deleting { owner_token: u64 },
    Deleted,
}

impl ThreadCoordinationState {
    fn from_stable(state: StableState) -> Self {
        match state {
            StableState::Live => Self::Live,
            StableState::Archived => Self::Archived,
            StableState::Deleted => Self::Deleted,
        }
    }

    fn stable(self) -> Option<StableState> {
        match self {
            Self::Live => Some(StableState::Live),
            Self::Archived => Some(StableState::Archived),
            Self::Deleted => Some(StableState::Deleted),
            Self::Uncalibrated
            | Self::Deciding { .. }
            | Self::Archiving { .. }
            | Self::Deleting { .. } => None,
        }
    }

    fn owned(self) -> Option<(OwnedStateKind, u64)> {
        match self {
            Self::Deciding { owner_token } => Some((OwnedStateKind::Deciding, owner_token)),
            Self::Archiving { owner_token } => Some((OwnedStateKind::Archiving, owner_token)),
            Self::Deleting { owner_token } => Some((OwnedStateKind::Deleting, owner_token)),
            Self::Uncalibrated | Self::Live | Self::Archived | Self::Deleted => None,
        }
    }
}

struct LeaseEntry {
    valid: Arc<AtomicBool>,
    active: bool,
}

struct ThreadCoordinationEntry {
    epoch: u64,
    state: ThreadCoordinationState,
    leases: HashMap<u64, LeaseEntry>,
    /// Calibration, decision, and mutation tokens all live here. An entry is
    /// never normally evicted while any token remains unsettled.
    pending_tokens: HashSet<u64>,
}

impl ThreadCoordinationEntry {
    fn uncalibrated(epoch: u64) -> Self {
        Self {
            epoch,
            state: ThreadCoordinationState::Uncalibrated,
            leases: HashMap::new(),
            pending_tokens: HashSet::new(),
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
/// mutations.
///
/// Durable tombstones are the only source of truth. Entries begin in
/// `Uncalibrated` and are populated through a two-phase durable read. Every
/// installed state receives a process-global monotonic epoch so an old token
/// can never match an entry that was evicted and recreated.
pub struct ThreadRunCoordinator {
    entries: Mutex<HashMap<String, ThreadCoordinationEntry>>,
    aborter: Mutex<Option<Arc<dyn ThreadRunAborter>>>,
    next_nonce: AtomicU64,
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
            next_nonce: AtomicU64::new(0),
            changed: Notify::new(),
        }
    }

    pub fn shared_fallback() -> Arc<Self> {
        static COORDINATOR: OnceLock<Arc<ThreadRunCoordinator>> = OnceLock::new();
        COORDINATOR
            .get_or_init(|| Arc::new(ThreadRunCoordinator::new()))
            .clone()
    }

    fn nonce(&self) -> u64 {
        self.next_nonce
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .expect("thread coordinator nonce exhausted")
            + 1
    }

    pub fn set_aborter(&self, aborter: Arc<dyn ThreadRunAborter>) {
        *self
            .aborter
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(aborter);
    }

    /// Two-phase read-through calibration. Neither the coordinator mutex nor
    /// any registry mutex is held across the durable store await.
    async fn ensure_calibrated(
        &self,
        store: &dyn ThreadStore,
        thread_id: &str,
    ) -> Result<(), RunAdmissionError> {
        loop {
            let (expected_epoch, calibration_token) = {
                let mut entries = self
                    .entries
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                let initial_epoch = self.nonce();
                let entry = entries
                    .entry(thread_id.to_owned())
                    .or_insert_with(|| ThreadCoordinationEntry::uncalibrated(initial_epoch));
                if entry.state != ThreadCoordinationState::Uncalibrated {
                    return Ok(());
                }
                let token = self.nonce();
                entry.pending_tokens.insert(token);
                (entry.epoch, token)
            };

            let durable = store.terminal_state(thread_id).await;
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let mut remove_failed_entry = false;
            if let Some(entry) = entries.get_mut(thread_id) {
                if let Ok(terminal) = &durable
                    && entry.epoch == expected_epoch
                    && entry.state == ThreadCoordinationState::Uncalibrated
                {
                    entry.epoch = self.nonce();
                    entry.state =
                        ThreadCoordinationState::from_stable(StableState::from_terminal(*terminal));
                }
                entry.pending_tokens.remove(&calibration_token);
                remove_failed_entry = durable.is_err()
                    && entry.state == ThreadCoordinationState::Uncalibrated
                    && entry.leases.is_empty()
                    && entry.pending_tokens.is_empty();
            }
            if remove_failed_entry {
                entries.remove(thread_id);
            }
            drop(entries);
            self.changed.notify_waiters();

            durable.map_err(|error| RunAdmissionError::Store(error.to_string()))?;
            // Another state installation may have won while the durable read
            // was in flight. Loop and consume that newer state.
        }
    }

    async fn reserve_request(
        self: &Arc<Self>,
        store: &dyn ThreadStore,
        thread_id: &str,
    ) -> Result<ThreadRunLease, RunAdmissionError> {
        self.ensure_calibrated(store, thread_id).await?;
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(entry) = entries.get_mut(thread_id) else {
            return Err(RunAdmissionError::Stale(thread_id.to_owned()));
        };
        match entry.state {
            ThreadCoordinationState::Live => {}
            ThreadCoordinationState::Archived | ThreadCoordinationState::Deleted => {
                return Err(RunAdmissionError::Archived(thread_id.to_owned()));
            }
            ThreadCoordinationState::Uncalibrated
            | ThreadCoordinationState::Deciding { .. }
            | ThreadCoordinationState::Archiving { .. }
            | ThreadCoordinationState::Deleting { .. } => {
                return Err(RunAdmissionError::Stale(thread_id.to_owned()));
            }
        }
        let lease_id = self.nonce();
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
            .is_some_and(|entry| entry.state.owned().is_some())
    }

    pub async fn reserve_archive(
        self: &Arc<Self>,
        store: &dyn ThreadStore,
        thread_id: &str,
    ) -> Result<ArchiveBarrier, CoordinationError> {
        self.ensure_calibrated(store, thread_id)
            .await
            .map_err(CoordinationError::from_admission)?;
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let entry = entries
            .get_mut(thread_id)
            .ok_or(CoordinationError::Unavailable)?;
        let Some(prior) = entry.state.stable() else {
            return Err(CoordinationError::Unavailable);
        };

        // Persistent terminal truth wins over any short-lived inactive lease
        // created before calibration completed. Only a live active run is a
        // deterministic archive conflict.
        if prior == StableState::Live && entry.leases.values().any(|lease| lease.active) {
            let reservation =
                self.install_reservation(thread_id, entry, prior, OwnedStateKind::Deciding);
            return Ok(ArchiveBarrier::ActiveLease(reservation));
        }

        for lease in entry.leases.values() {
            lease.valid.store(false, Ordering::Release);
        }
        let reservation =
            self.install_reservation(thread_id, entry, prior, OwnedStateKind::Archiving);
        drop(entries);
        self.changed.notify_waiters();
        Ok(ArchiveBarrier::Ready(reservation))
    }

    /// Reserve a short decision transaction without performing destructive
    /// run invalidation. Used after delete binding preflight rejects.
    pub async fn reserve_decision(
        self: &Arc<Self>,
        store: &dyn ThreadStore,
        thread_id: &str,
    ) -> Result<LifecycleReservation, CoordinationError> {
        self.ensure_calibrated(store, thread_id)
            .await
            .map_err(CoordinationError::from_admission)?;
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let entry = entries
            .get_mut(thread_id)
            .ok_or(CoordinationError::Unavailable)?;
        let prior = entry.state.stable().ok_or(CoordinationError::Unavailable)?;
        Ok(self.install_reservation(thread_id, entry, prior, OwnedStateKind::Deciding))
    }

    /// Fence a caller-reserved thread ID before its initial truth record is
    /// published. The reservation can be atomically transferred to the first
    /// request lease after the database create commit.
    pub async fn reserve_creation(
        self: &Arc<Self>,
        store: &dyn ThreadStore,
        thread_id: &str,
    ) -> Result<LifecycleReservation, CoordinationError> {
        if !is_thread_key(thread_id) {
            return Err(CoordinationError::Store(format!(
                "invalid canonical thread id: {thread_id}"
            )));
        }
        self.ensure_calibrated(store, thread_id)
            .await
            .map_err(CoordinationError::from_admission)?;
        if store
            .get(thread_id)
            .await
            .map_err(|error| CoordinationError::Store(error.to_string()))?
            .is_some()
        {
            return Err(CoordinationError::Unavailable);
        }
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let entry = entries
            .get_mut(thread_id)
            .ok_or(CoordinationError::Unavailable)?;
        let prior = entry.state.stable().ok_or(CoordinationError::Unavailable)?;
        if prior != StableState::Live || !entry.leases.is_empty() {
            return Err(CoordinationError::Unavailable);
        }
        Ok(self.install_reservation(thread_id, entry, prior, OwnedStateKind::Deciding))
    }

    pub async fn reserve_delete(
        self: &Arc<Self>,
        store: &dyn ThreadStore,
        thread_id: &str,
    ) -> Result<LifecycleReservation, CoordinationError> {
        self.ensure_calibrated(store, thread_id)
            .await
            .map_err(CoordinationError::from_admission)?;
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let entry = entries
            .get_mut(thread_id)
            .ok_or(CoordinationError::Unavailable)?;
        let prior = entry.state.stable().ok_or(CoordinationError::Unavailable)?;
        for lease in entry.leases.values() {
            lease.valid.store(false, Ordering::Release);
        }
        let reservation =
            self.install_reservation(thread_id, entry, prior, OwnedStateKind::Deleting);
        drop(entries);
        self.changed.notify_waiters();
        Ok(reservation)
    }

    fn install_reservation(
        self: &Arc<Self>,
        thread_id: &str,
        entry: &mut ThreadCoordinationEntry,
        prior: StableState,
        kind: OwnedStateKind,
    ) -> LifecycleReservation {
        let owner_token = self.nonce();
        let epoch = self.nonce();
        entry.epoch = epoch;
        entry.state = match kind {
            OwnedStateKind::Deciding => ThreadCoordinationState::Deciding { owner_token },
            OwnedStateKind::Archiving => ThreadCoordinationState::Archiving { owner_token },
            OwnedStateKind::Deleting => ThreadCoordinationState::Deleting { owner_token },
        };
        entry.pending_tokens.insert(owner_token);
        LifecycleReservation {
            coordinator: Arc::clone(self),
            token: ReservationToken {
                thread_id: thread_id.to_owned(),
                epoch,
                expected_state: kind,
                owner_token,
            },
            prior,
            commit_witness: LifecycleCommitWitness::default(),
            settled: false,
        }
    }

    pub async fn abort_and_drain_delete(
        &self,
        reservation: &LifecycleReservation,
    ) -> Result<(), CoordinationError> {
        self.abort_and_drain_token(&reservation.token).await
    }

    async fn abort_and_drain_token(
        &self,
        token: &ReservationToken,
    ) -> Result<(), CoordinationError> {
        if token.expected_state != OwnedStateKind::Deleting || !self.reservation_is_current(token) {
            return Err(CoordinationError::Unavailable);
        }
        let aborter = self
            .aborter
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
            .cloned();
        if let Some(aborter) = aborter {
            aborter
                .abort_and_drain_thread(&token.thread_id)
                .await
                .map_err(CoordinationError::Abort)?;
        }
        self.wait_for_no_leases(token).await
    }

    async fn wait_for_no_leases(&self, token: &ReservationToken) -> Result<(), CoordinationError> {
        loop {
            let notified = self.changed.notified();
            let status = {
                let entries = self
                    .entries
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                let Some(entry) = entries.get(&token.thread_id) else {
                    return Err(CoordinationError::Unavailable);
                };
                if !token.matches(entry) {
                    return Err(CoordinationError::Unavailable);
                }
                entry.leases.is_empty()
            };
            if status {
                return Ok(());
            }
            notified.await;
        }
    }

    fn reservation_is_current(&self, token: &ReservationToken) -> bool {
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&token.thread_id)
            .is_some_and(|entry| token.matches(entry))
    }

    fn release_lease(&self, thread_id: &str, lease_id: u64) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let remove = if let Some(entry) = entries.get_mut(thread_id) {
            entry.leases.remove(&lease_id);
            entry.state == ThreadCoordinationState::Live
                && entry.leases.is_empty()
                && entry.pending_tokens.is_empty()
        } else {
            false
        };
        if remove {
            entries.remove(thread_id);
        }
        drop(entries);
        self.changed.notify_waiters();
    }

    fn settle_reservation(&self, token: &ReservationToken, durable: StableState) -> bool {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let mut matched = false;
        let mut remove = false;
        if let Some(entry) = entries.get_mut(&token.thread_id) {
            if token.matches(entry) {
                matched = true;
                entry.pending_tokens.remove(&token.owner_token);
                entry.epoch = self.nonce();
                entry.state = ThreadCoordinationState::from_stable(durable);
                remove = durable == StableState::Live
                    && entry.leases.is_empty()
                    && entry.pending_tokens.is_empty();
            } else {
                // Defensive cleanup is safe because owner tokens are globally
                // unique. It never changes a newer entry's state.
                entry.pending_tokens.remove(&token.owner_token);
            }
        }
        if remove {
            entries.remove(&token.thread_id);
        }
        drop(entries);
        self.changed.notify_waiters();
        matched
    }

    /// A successful ordinary write cannot clear a durable terminal tombstone.
    /// The hook remains during the step-1 migration so existing stores compile;
    /// it intentionally performs no terminal-cache mutation.
    pub fn record_written(&self, _thread_id: &str) {}

    #[cfg(test)]
    fn force_evict_for_test(&self, thread_id: &str) {
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(thread_id);
        self.changed.notify_waiters();
    }

    #[cfg(test)]
    fn entry_epoch_for_test(&self, thread_id: &str) -> Option<u64> {
        self.entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(thread_id)
            .map(|entry| entry.epoch)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoordinationError {
    #[error("thread lifecycle mutation is already in progress")]
    Unavailable,
    #[error("thread store backend failed: {0}")]
    Store(String),
    #[error("failed to abort and drain thread runs: {0}")]
    Abort(String),
}

impl CoordinationError {
    fn from_admission(error: RunAdmissionError) -> Self {
        match error {
            RunAdmissionError::Store(message) => Self::Store(message),
            other => Self::Store(other.to_string()),
        }
    }
}

pub enum ArchiveBarrier {
    Ready(LifecycleReservation),
    ActiveLease(LifecycleReservation),
}

#[derive(Debug, Clone)]
struct ReservationToken {
    thread_id: String,
    epoch: u64,
    expected_state: OwnedStateKind,
    owner_token: u64,
}

impl ReservationToken {
    fn matches(&self, entry: &ThreadCoordinationEntry) -> bool {
        entry.epoch == self.epoch
            && entry.state.owned() == Some((self.expected_state, self.owner_token))
    }
}

/// RAII owner token for a coordinator decision or lifecycle mutation.
///
/// A caller must mark a durable commit synchronously after SQLite returns.
/// If it panics before explicit settlement, Drop applies that committed state;
/// otherwise Drop restores the calibrated prior state through the same full
/// tuple CAS.
pub struct LifecycleReservation {
    coordinator: Arc<ThreadRunCoordinator>,
    token: ReservationToken,
    prior: StableState,
    commit_witness: LifecycleCommitWitness,
    settled: bool,
}

/// Cloneable commit marker handed to an owned blocking child. The child marks
/// it immediately after SQLite returns a committed lifecycle/decision result,
/// before its JoinHandle becomes ready. If the supervisor is aborted, the
/// reaper-held reservation observes this marker in Drop and applies durable
/// state instead of restoring its prior cache snapshot.
#[derive(Clone, Default)]
pub struct LifecycleCommitWitness {
    durable_terminal: Arc<Mutex<Option<Option<ThreadTerminalState>>>>,
}

impl LifecycleCommitWitness {
    pub fn mark_committed(&self, durable_terminal: Option<ThreadTerminalState>) {
        *self
            .durable_terminal
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(durable_terminal);
    }

    fn committed_terminal(&self) -> Option<Option<ThreadTerminalState>> {
        *self
            .durable_terminal
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

impl LifecycleReservation {
    pub fn thread_id(&self) -> &str {
        &self.token.thread_id
    }

    pub fn prior_terminal(&self) -> Option<ThreadTerminalState> {
        self.prior.terminal()
    }

    pub fn is_current(&self) -> bool {
        self.coordinator.reservation_is_current(&self.token)
    }

    pub fn commit_witness(&self) -> LifecycleCommitWitness {
        self.commit_witness.clone()
    }

    /// Owned drain future for a mutation supervisor. The reservation remains
    /// guard-owned while the independently spawned future carries only its
    /// immutable full-tuple token, so cancellation cannot release the fence
    /// ahead of an abort/drain descendant.
    pub fn abort_and_drain_future(
        &self,
    ) -> impl Future<Output = Result<(), CoordinationError>> + Send + 'static {
        let coordinator = Arc::clone(&self.coordinator);
        let token = self.token.clone();
        async move { coordinator.abort_and_drain_token(&token).await }
    }

    pub fn mark_committed(&mut self, durable_terminal: Option<ThreadTerminalState>) {
        self.commit_witness.mark_committed(durable_terminal);
    }

    pub fn settle_committed(&mut self, durable_terminal: Option<ThreadTerminalState>) {
        self.mark_committed(durable_terminal);
        self.settle_to(StableState::from_terminal(durable_terminal));
    }

    pub fn settle_decision(&mut self, durable_terminal: Option<ThreadTerminalState>) {
        self.settle_to(StableState::from_terminal(durable_terminal));
    }

    pub fn settle_transient(&mut self, durable_terminal: Option<ThreadTerminalState>) {
        self.settle_to(StableState::from_terminal(durable_terminal));
    }

    fn settle_to(&mut self, durable: StableState) {
        if !self.settled {
            self.coordinator.settle_reservation(&self.token, durable);
            self.settled = true;
        }
    }

    fn transfer_to_request_lease(mut self) -> Result<ThreadRunLease, RunAdmissionError> {
        let mut entries = self
            .coordinator
            .entries
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let lease_id = self.coordinator.nonce();
        let valid = Arc::new(AtomicBool::new(true));
        let Some(entry) = entries.get_mut(&self.token.thread_id) else {
            return Err(RunAdmissionError::Stale(self.token.thread_id.clone()));
        };
        if !self.token.matches(entry) || self.prior != StableState::Live {
            return Err(RunAdmissionError::Stale(self.token.thread_id.clone()));
        }
        entry.pending_tokens.remove(&self.token.owner_token);
        entry.epoch = self.coordinator.nonce();
        entry.state = ThreadCoordinationState::Live;
        entry.leases.insert(
            lease_id,
            LeaseEntry {
                valid: valid.clone(),
                active: false,
            },
        );
        self.commit_witness.mark_committed(None);
        self.settled = true;
        drop(entries);
        self.coordinator.changed.notify_waiters();
        Ok(ThreadRunLease {
            coordinator: Arc::clone(&self.coordinator),
            thread_id: self.token.thread_id.clone(),
            lease_id,
            valid,
            released: false,
        })
    }
}

impl Drop for LifecycleReservation {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let durable = self
            .commit_witness
            .committed_terminal()
            .map(StableState::from_terminal)
            .unwrap_or(self.prior);
        self.coordinator.settle_reservation(&self.token, durable);
        self.settled = true;
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
        if entry.state != ThreadCoordinationState::Live {
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

    /// Point-read the backing store and acquire a calibrated request lease.
    /// Callers cannot supply a pre-read record as proof.
    pub async fn thread_bound(
        store: Arc<dyn ThreadStore>,
        request: AgentRunRequest,
    ) -> Result<Self, RunAdmissionError> {
        if !is_thread_key(&request.thread_id) {
            return Err(RunAdmissionError::InvalidThreadId(request.thread_id));
        }
        let coordinator = store.run_coordinator();
        let lease = coordinator
            .reserve_request(store.as_ref(), &request.thread_id)
            .await?;
        let exists = store
            .get(&request.thread_id)
            .await
            .map_err(|error| RunAdmissionError::Store(error.to_string()))?
            .is_some();
        if !exists {
            return Err(RunAdmissionError::NotFound(request.thread_id));
        }
        lease.ensure_valid()?;
        Ok(Self {
            inner: AdmittedRunInner::ThreadBound { request, lease },
        })
    }

    /// Seal the first run for a newly committed thread by transferring the
    /// pre-commit creation reservation directly into a request lease. No
    /// lifecycle or competing request can interleave between those states.
    pub async fn thread_bound_from_creation(
        store: Arc<dyn ThreadStore>,
        request: AgentRunRequest,
        reservation: LifecycleReservation,
    ) -> Result<Self, RunAdmissionError> {
        if !is_thread_key(&request.thread_id) {
            return Err(RunAdmissionError::InvalidThreadId(request.thread_id));
        }
        if reservation.thread_id() != request.thread_id {
            return Err(RunAdmissionError::Stale(request.thread_id));
        }
        let exists = store
            .get(&request.thread_id)
            .await
            .map_err(|error| RunAdmissionError::Store(error.to_string()))?
            .is_some();
        if !exists {
            return Err(RunAdmissionError::NotFound(request.thread_id));
        }
        let lease = reservation.transfer_to_request_lease()?;
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
    use crate::{AtomicRecordMerge, InMemoryThreadStore, ThreadStoreError};
    use garyx_models::provider::ProviderType;
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    fn request(thread_id: &str) -> AgentRunRequest {
        AgentRunRequest::new(thread_id, "hello", "run::1", "api", "main", HashMap::new())
    }

    async fn store_with_thread(thread_id: &str) -> Arc<dyn ThreadStore> {
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
        store.set(thread_id, json!({})).await.unwrap();
        store
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
    }

    #[tokio::test]
    async fn thread_bound_admission_requires_a_real_record_and_drop_releases_lease() {
        let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
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
    async fn calibrated_archive_distinguishes_active_from_inactive_lease() {
        let thread_id = "thread::archive-conflict";
        let store = store_with_thread(thread_id).await;
        let admitted = AdmittedRun::thread_bound(store.clone(), request(thread_id))
            .await
            .unwrap();
        let (_request, mut lease) = admitted.into_dispatch_parts();
        let mut lease = lease.take().unwrap();
        lease.promote_to_active().unwrap();

        let barrier = store
            .run_coordinator()
            .reserve_archive(store.as_ref(), thread_id)
            .await
            .unwrap();
        let ArchiveBarrier::ActiveLease(mut decision) = barrier else {
            panic!("live active lease must take the persistent decision path");
        };
        assert!(
            lease.ensure_valid().is_ok(),
            "archive conflict must not kill the run"
        );
        decision.settle_decision(None);
        drop(lease);

        let admitted = AdmittedRun::thread_bound(store.clone(), request(thread_id))
            .await
            .unwrap();
        let (_request, lease) = admitted.into_dispatch_parts();
        let lease = lease.unwrap();
        let barrier = store
            .run_coordinator()
            .reserve_archive(store.as_ref(), thread_id)
            .await
            .unwrap();
        let ArchiveBarrier::Ready(mut reservation) = barrier else {
            panic!("an inactive request token is not an active-run conflict");
        };
        assert!(matches!(
            lease.ensure_valid(),
            Err(RunAdmissionError::Stale(_))
        ));
        reservation.settle_transient(None);
    }

    struct SequencedTerminalStore {
        inner: InMemoryThreadStore,
        calls: AtomicUsize,
        first_started: Notify,
        release_first: Notify,
        first_terminal: Option<ThreadTerminalState>,
        later_terminal: Option<ThreadTerminalState>,
    }

    impl SequencedTerminalStore {
        async fn with_record(
            thread_id: &str,
            first_terminal: Option<ThreadTerminalState>,
            later_terminal: Option<ThreadTerminalState>,
        ) -> Arc<Self> {
            let store = Arc::new(Self {
                inner: InMemoryThreadStore::new(),
                calls: AtomicUsize::new(0),
                first_started: Notify::new(),
                release_first: Notify::new(),
                first_terminal,
                later_terminal,
            });
            store.inner.set(thread_id, json!({})).await.unwrap();
            store
        }
    }

    #[async_trait]
    impl ThreadStore for SequencedTerminalStore {
        fn run_coordinator(&self) -> Arc<ThreadRunCoordinator> {
            self.inner.run_coordinator()
        }

        async fn terminal_state(
            &self,
            _thread_id: &str,
        ) -> Result<Option<ThreadTerminalState>, ThreadStoreError> {
            let call = self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            if call == 0 {
                let snapshot = self.first_terminal;
                self.first_started.notify_one();
                self.release_first.notified().await;
                Ok(snapshot)
            } else {
                Ok(self.later_terminal)
            }
        }

        async fn get(&self, thread_id: &str) -> Result<Option<Value>, ThreadStoreError> {
            self.inner.get(thread_id).await
        }
        async fn set(&self, thread_id: &str, data: Value) -> Result<(), ThreadStoreError> {
            self.inner.set(thread_id, data).await
        }
        async fn delete(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
            self.inner.delete(thread_id).await
        }
        async fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>, ThreadStoreError> {
            self.inner.list_keys(prefix).await
        }
        async fn exists(&self, thread_id: &str) -> Result<bool, ThreadStoreError> {
            self.inner.exists(thread_id).await
        }
        async fn update(&self, thread_id: &str, updates: Value) -> Result<(), ThreadStoreError> {
            self.inner.update(thread_id, updates).await
        }
        async fn update_many_atomic(
            &self,
            entries: Vec<AtomicRecordMerge>,
        ) -> Result<(), ThreadStoreError> {
            self.inner.update_many_atomic(entries).await
        }
    }

    #[tokio::test]
    async fn stale_calibration_snapshot_cannot_overwrite_a_new_archive_reservation() {
        let thread_id = "thread::calibration-fence";
        let store = SequencedTerminalStore::with_record(
            thread_id,
            Some(ThreadTerminalState::Deleted),
            None,
        )
        .await;
        let first_store: Arc<dyn ThreadStore> = store.clone();
        let first = tokio::spawn(async move {
            AdmittedRun::thread_bound(first_store, request(thread_id)).await
        });
        store.first_started.notified().await;

        let coordinator = store.run_coordinator();
        let barrier = coordinator
            .reserve_archive(store.as_ref(), thread_id)
            .await
            .unwrap();
        let ArchiveBarrier::Ready(mut reservation) = barrier else {
            panic!("second calibration should install live then reserve archive");
        };
        store.release_first.notify_one();
        assert!(matches!(
            first.await.unwrap(),
            Err(RunAdmissionError::Stale(_))
        ));
        assert!(
            reservation.is_current(),
            "stale durable snapshot overwrote reservation"
        );
        reservation.settle_transient(None);
    }

    #[tokio::test]
    async fn pending_calibration_token_prevents_last_lease_eviction() {
        let thread_id = "thread::pending-token";
        let store = SequencedTerminalStore::with_record(thread_id, None, None).await;
        let first_store: Arc<dyn ThreadStore> = store.clone();
        let first = tokio::spawn(async move {
            AdmittedRun::thread_bound(first_store, request(thread_id)).await
        });
        store.first_started.notified().await;

        let second_store: Arc<dyn ThreadStore> = store.clone();
        let second = AdmittedRun::thread_bound(second_store, request(thread_id))
            .await
            .unwrap();
        let coordinator = store.run_coordinator();
        let epoch = coordinator.entry_epoch_for_test(thread_id).unwrap();
        drop(second);
        assert_eq!(
            coordinator.entry_epoch_for_test(thread_id),
            Some(epoch),
            "last lease release evicted an entry with an unsettled calibration token"
        );

        store.release_first.notify_one();
        drop(first.await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn global_epoch_rejects_old_completion_after_forced_entry_eviction() {
        let thread_id = "thread::epoch-aba";
        let store = store_with_thread(thread_id).await;
        let coordinator = store.run_coordinator();
        let ArchiveBarrier::Ready(mut old) = coordinator
            .reserve_archive(store.as_ref(), thread_id)
            .await
            .unwrap()
        else {
            panic!("archive reservation expected");
        };
        let old_epoch = coordinator.entry_epoch_for_test(thread_id).unwrap();
        coordinator.force_evict_for_test(thread_id);

        let fresh = coordinator
            .reserve_delete(store.as_ref(), thread_id)
            .await
            .unwrap();
        let fresh_epoch = coordinator.entry_epoch_for_test(thread_id).unwrap();
        assert!(fresh_epoch > old_epoch, "entry recreation reused an epoch");
        old.settle_transient(None);
        assert!(
            fresh.is_current(),
            "old completion overwrote the fresh reservation"
        );
        drop(fresh);
    }

    #[tokio::test]
    async fn dropped_committed_reservation_applies_durable_terminal() {
        let thread_id = "thread::commit-drop";
        let store = store_with_thread(thread_id).await;
        let coordinator = store.run_coordinator();
        let ArchiveBarrier::Ready(mut reservation) = coordinator
            .reserve_archive(store.as_ref(), thread_id)
            .await
            .unwrap()
        else {
            panic!("archive reservation expected");
        };
        reservation.mark_committed(Some(ThreadTerminalState::Archived));
        drop(reservation);
        assert!(matches!(
            AdmittedRun::thread_bound(store, request(thread_id)).await,
            Err(RunAdmissionError::Archived(_))
        ));
    }

    #[tokio::test]
    async fn decision_and_transient_completion_restore_calibrated_terminal_state() {
        let deleted_id = "thread::deleted-decision";
        let deleted = Arc::new(InMemoryThreadStore::new());
        deleted
            .seed_terminal_state(deleted_id, ThreadTerminalState::Deleted)
            .await;
        let deleted_store: Arc<dyn ThreadStore> = deleted;
        let ArchiveBarrier::Ready(mut archive) = deleted_store
            .run_coordinator()
            .reserve_archive(deleted_store.as_ref(), deleted_id)
            .await
            .unwrap()
        else {
            panic!("terminal state must reach the durable result matrix");
        };
        assert_eq!(archive.prior_terminal(), Some(ThreadTerminalState::Deleted));
        archive.settle_decision(Some(ThreadTerminalState::Deleted));
        assert!(matches!(
            AdmittedRun::thread_bound(deleted_store, request(deleted_id)).await,
            Err(RunAdmissionError::Archived(_))
        ));

        let archived_id = "thread::archived-transient";
        let archived = Arc::new(InMemoryThreadStore::new());
        archived
            .seed_terminal_state(archived_id, ThreadTerminalState::Archived)
            .await;
        let archived_store: Arc<dyn ThreadStore> = archived;
        let mut delete = archived_store
            .run_coordinator()
            .reserve_delete(archived_store.as_ref(), archived_id)
            .await
            .unwrap();
        assert_eq!(delete.prior_terminal(), Some(ThreadTerminalState::Archived));
        delete.settle_transient(Some(ThreadTerminalState::Archived));
        assert!(matches!(
            AdmittedRun::thread_bound(archived_store, request(archived_id)).await,
            Err(RunAdmissionError::Archived(_))
        ));
    }
}
