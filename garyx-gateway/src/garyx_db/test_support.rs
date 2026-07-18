//! Test-only fault injection and mutation barriers (`test-seams`).

use super::*;

#[cfg(any(test, feature = "test-seams"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TestDbFaultPoint {
    LegacyMarkerPairRead,
    LegacyImportCommit,
    LegacyImportAfterIncarnationRotation,
    LegacyRetirementMarkerWrite,
    ArchivedThreadRead,
    LegacyGenerationSeedWrite,
    DeleteThreadRecord,
}

#[cfg(any(test, feature = "test-seams"))]
#[derive(Debug, Default)]
pub(super) struct TestDbFaults {
    calls: HashMap<TestDbFaultPoint, usize>,
    fail_on: HashSet<(TestDbFaultPoint, usize)>,
    mutation_barriers: HashMap<TestDbMutationPoint, std::sync::Arc<TestDbMutationBarrierState>>,
}

#[cfg(any(test, feature = "test-seams"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TestDbMutationPoint {
    ArchiveThreadRecord,
    DeleteThreadRecord,
}

#[cfg(any(test, feature = "test-seams"))]
#[derive(Debug)]
pub(super) struct TestDbMutationBarrierState {
    started: AtomicBool,
    started_notify: tokio::sync::Notify,
    released: Mutex<bool>,
    release_notify: Condvar,
}

#[cfg(any(test, feature = "test-seams"))]
impl TestDbMutationBarrierState {
    fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
            started_notify: tokio::sync::Notify::new(),
            released: Mutex::new(false),
            release_notify: Condvar::new(),
        }
    }

    fn block(&self) {
        self.started.store(true, Ordering::Release);
        self.started_notify.notify_waiters();
        let mut released = self
            .released
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while !*released {
            released = self
                .release_notify
                .wait(released)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    fn release(&self) {
        *self
            .released
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        self.release_notify.notify_all();
    }
}

/// One-shot deterministic seam proving that a coordinator-owned blocking
/// mutation outlives cancellation of the HTTP future that initiated it.
#[cfg(any(test, feature = "test-seams"))]
pub(crate) struct TestDbMutationBarrier {
    state: std::sync::Arc<TestDbMutationBarrierState>,
}

#[cfg(any(test, feature = "test-seams"))]
impl TestDbMutationBarrier {
    pub(crate) async fn wait_until_started(&self) {
        loop {
            let notified = self.state.started_notify.notified();
            if self.state.started.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }

    pub(crate) fn release(&self) {
        self.state.release();
    }
}

#[cfg(any(test, feature = "test-seams"))]
impl Drop for TestDbMutationBarrier {
    fn drop(&mut self) {
        // A failed test must never strand a blocking-pool worker.
        self.state.release();
    }
}

impl GaryxDbService {
    #[cfg(any(test, feature = "test-seams"))]
    pub(crate) fn fail_test_db_call(&self, point: TestDbFaultPoint, occurrence: usize) {
        assert!(occurrence > 0, "fault occurrence is one-based");
        self.test_faults
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .fail_on
            .insert((point, occurrence));
    }

    #[cfg(any(test, feature = "test-seams"))]
    pub(crate) fn block_test_db_mutation(
        &self,
        point: TestDbMutationPoint,
    ) -> TestDbMutationBarrier {
        let state = std::sync::Arc::new(TestDbMutationBarrierState::new());
        self.test_faults
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .mutation_barriers
            .insert(point, state.clone());
        TestDbMutationBarrier { state }
    }

    #[cfg(any(test, feature = "test-seams"))]
    pub(super) fn maybe_block_test_db_mutation(&self, point: TestDbMutationPoint) {
        let barrier = self
            .test_faults
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .mutation_barriers
            .remove(&point);
        if let Some(barrier) = barrier {
            barrier.block();
        }
    }

    #[cfg(any(test, feature = "test-seams"))]
    pub(super) fn maybe_fail_test_db_call(&self, point: TestDbFaultPoint) -> GaryxDbResult<()> {
        let mut faults = self
            .test_faults
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let occurrence = {
            let calls = faults.calls.entry(point).or_default();
            *calls += 1;
            *calls
        };
        if faults.fail_on.remove(&(point, occurrence)) {
            return Err(GaryxDbError::Configuration(format!(
                "injected database fault at {point:?} call {occurrence}"
            )));
        }
        Ok(())
    }
}
