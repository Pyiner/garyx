import XCTest
@testable import GaryxMobileCore

final class HomeProjectionReducerTests: XCTestCase {
    func testReducerCheckpointParityMatchesExistingHomeListStoreAcrossCorpora() {
        let corpora = [
            GaryxHomeListFixture.makeInputs(threadCount: 12, pinnedCount: 3, runningCount: 0),
            GaryxHomeListFixture.makeInputs(threadCount: 50, pinnedCount: 6, runningCount: 4),
            GaryxHomeListFixture.makeInputs(threadCount: 120, pinnedCount: 10, runningCount: 8),
        ]

        for corpus in corpora {
            var state = HomeProjectionState()
            let store = GaryxHomeThreadListStore()

            state = reduce(state, ingest(corpus, epoch: 1)).state
            assertCheckpointParity(state, store, file: #filePath, line: #line)

            state = reduce(state, .loadingChanged(isLoading: true)).state
            state = reduce(state, .homeVisibilityChanged(isVisible: true)).state
            assertCheckpointParity(state, store, file: #filePath, line: #line)

            state = reduce(state, .selectedThreadChanged(threadId: "thread-3")).state
            assertCheckpointParity(state, store, file: #filePath, line: #line)

            state = reduce(
                state,
                .runStateDelta(
                    source: .runTracker,
                    threadId: "thread-7",
                    status: .running,
                    basedOnSeq: 1
                )
            ).state
            state = reduce(
                state,
                .runStateDelta(
                    source: .committedRunState,
                    threadId: "thread-8",
                    status: .running,
                    basedOnSeq: 20
                )
            ).state
            assertCheckpointParity(state, store, file: #filePath, line: #line)

            let pinned = ["thread-7", "thread-0", "thread-1"]
            state = reduce(state, .pinsChanged(pinnedThreadIds: pinned)).state
            assertCheckpointParity(state, store, file: #filePath, line: #line)
        }
    }

    func testRunOnlyChurnDoesNotRebuildOrEvaluateDisplayIdentity() throws {
        let input = GaryxHomeListFixture.makeInputs(threadCount: 60, pinnedCount: 4, runningCount: 0)
        var state = reduce(HomeProjectionState(), ingest(input, epoch: 1)).state
        XCTAssertEqual(state.baseSectionBuildCount, 1)
        XCTAssertEqual(state.displayRebuildEventCount, 1)
        XCTAssertEqual(state.sectionIdentityEvaluationCount, 0)
        XCTAssertEqual(state.rowDifferenceEvaluationCount, 1)

        for tick in 0..<300 {
            state = reduce(
                state,
                .runStateDelta(
                    source: .runTracker,
                    threadId: "thread-10",
                    status: tick.isMultiple(of: 2) ? .running : .idle,
                    basedOnSeq: tick + 1
                )
            ).state
        }

        XCTAssertEqual(state.baseSectionBuildCount, 1)
        XCTAssertEqual(state.displayRebuildEventCount, 1)
        XCTAssertEqual(
            state.sectionIdentityEvaluationCount,
            0,
            "Run-only events must not construct the old O(N) section identity key."
        )
        XCTAssertEqual(
            state.rowDifferenceEvaluationCount,
            1,
            "Run-only events must not scan all row ids to compute a collection diff."
        )
        XCTAssertEqual(state.runStatePatchCount, 300)
        let row = try XCTUnwrap(state.snapshot.sections.recent.first { $0.id == "thread-10" })
        XCTAssertFalse(row.presentation.isRunning)
    }

    func testRepeatedIdenticalRecentIngestDoesNotRebuildBaseSections() {
        let input = GaryxHomeListFixture.makeInputs(threadCount: 40, pinnedCount: 5, runningCount: 2)
        var state = HomeProjectionState()

        for epoch in 1...20 {
            state = reduce(state, ingest(input, epoch: epoch)).state
        }

        XCTAssertEqual(state.baseSectionBuildCount, 1)
        XCTAssertEqual(state.displayRebuildEventCount, 1)
        XCTAssertEqual(state.snapshot.sections.allRows.count, 40)
    }

    func testSourcePrecedenceSuppressesStaleRecentRunningAfterHigherPriorityIdle() throws {
        let input = GaryxHomeListFixture.makeInputs(threadCount: 20, pinnedCount: 2, runningCount: 0)
        var state = reduce(HomeProjectionState(), ingest(input, epoch: 1)).state

        state = reduce(
            state,
            .runStateDelta(
                source: .recentThreadSummary,
                threadId: "thread-10",
                status: .running,
                basedOnSeq: 2
            )
        ).state
        XCTAssertTrue(try row(in: state, id: "thread-10").presentation.isRunning)

        state = reduce(
            state,
            .runStateDelta(
                source: .committedRunState,
                threadId: "thread-10",
                status: .idle,
                basedOnSeq: 30
            )
        ).state
        XCTAssertFalse(
            try row(in: state, id: "thread-10").presentation.isRunning,
            "A fresh committed idle must suppress a lower-priority recent running projection."
        )

        state = reduce(
            state,
            .runStateDelta(
                source: .recentThreadSummary,
                threadId: "thread-10",
                status: .running,
                basedOnSeq: 31
            )
        ).state
        XCTAssertFalse(
            try row(in: state, id: "thread-10").presentation.isRunning,
            "Lower-priority running must not re-light the dot while a higher-priority idle slot is fresh."
        )

        state = reduce(
            state,
            .runStateDelta(
                source: .committedRunState,
                threadId: "thread-10",
                status: .unknown,
                basedOnSeq: 32
            )
        ).state
        XCTAssertTrue(
            try row(in: state, id: "thread-10").presentation.isRunning,
            "Once the higher-priority slot is cleared, the lower-priority recent source may drive the dot."
        )
    }

    func testLowerPriorityIdleDoesNotClearHigherPriorityBusy() throws {
        let input = GaryxHomeListFixture.makeInputs(threadCount: 20, pinnedCount: 2, runningCount: 0)
        var state = reduce(HomeProjectionState(), ingest(input, epoch: 1)).state

        state = reduce(
            state,
            .runStateDelta(
                source: .runTracker,
                threadId: "thread-10",
                status: .running,
                basedOnSeq: 10
            )
        ).state
        state = reduce(
            state,
            .runStateDelta(
                source: .recentThreadSummary,
                threadId: "thread-10",
                status: .idle,
                basedOnSeq: 99
            )
        ).state

        XCTAssertTrue(
            try row(in: state, id: "thread-10").presentation.isRunning,
            "A low-priority idle projection must not clear local optimistic busy."
        )
    }

    func testHomeVisibleRunningDotComesFromRecentThreadProjection() throws {
        let input = GaryxHomeListFixture.makeInputs(threadCount: 12, pinnedCount: 0, runningCount: 1)
        var state = HomeProjectionState()
        state = reduce(state, ingest(input, epoch: 1)).state
        state = reduce(state, .selectedThreadChanged(threadId: "thread-0")).state
        state = reduce(state, .homeVisibilityChanged(isVisible: true)).state

        XCTAssertTrue(
            try row(in: state, id: "thread-0").presentation.isRunning,
            "B2 stops the selected-thread stream on home, so the home dot must remain supplied by recent_threads."
        )
    }

    func testSameSourceStaleFramesAreIgnored() throws {
        let input = GaryxHomeListFixture.makeInputs(threadCount: 20, pinnedCount: 2, runningCount: 0)
        var state = reduce(HomeProjectionState(), ingest(input, epoch: 1)).state

        state = reduce(
            state,
            .runStateDelta(
                source: .committedRunState,
                threadId: "thread-10",
                status: .running,
                basedOnSeq: 20
            )
        ).state
        state = reduce(
            state,
            .runStateDelta(
                source: .committedRunState,
                threadId: "thread-10",
                status: .idle,
                basedOnSeq: 19
            )
        ).state
        XCTAssertTrue(try row(in: state, id: "thread-10").presentation.isRunning)

        state = reduce(
            state,
            .runStateDelta(
                source: .committedRunState,
                threadId: "thread-10",
                status: .idle,
                basedOnSeq: 21
            )
        ).state
        XCTAssertFalse(try row(in: state, id: "thread-10").presentation.isRunning)
    }

    func testThreeSourceFoldUsesPriorityOnlyWhenHigherSourceHasFreshSlot() throws {
        let input = GaryxHomeListFixture.makeInputs(threadCount: 20, pinnedCount: 2, runningCount: 0)
        var state = reduce(HomeProjectionState(), ingest(input, epoch: 1)).state

        state = reduce(
            state,
            .runStateDelta(source: .recentThreadSummary, threadId: "thread-10", status: .running, basedOnSeq: 2)
        ).state
        XCTAssertTrue(try row(in: state, id: "thread-10").presentation.isRunning)

        state = reduce(
            state,
            .runStateDelta(source: .committedRunState, threadId: "thread-10", status: .idle, basedOnSeq: 3)
        ).state
        XCTAssertFalse(try row(in: state, id: "thread-10").presentation.isRunning)

        state = reduce(
            state,
            .runStateDelta(source: .runTracker, threadId: "thread-10", status: .running, basedOnSeq: 4)
        ).state
        XCTAssertTrue(try row(in: state, id: "thread-10").presentation.isRunning)

        state = reduce(
            state,
            .runStateDelta(source: .runTracker, threadId: "thread-10", status: .unknown, basedOnSeq: 5)
        ).state
        XCTAssertFalse(
            try row(in: state, id: "thread-10").presentation.isRunning,
            "After clearing runTracker, committed idle is the highest-priority fresh slot."
        )

        state = reduce(
            state,
            .runStateDelta(source: .committedRunState, threadId: "thread-10", status: .unknown, basedOnSeq: 6)
        ).state
        XCTAssertTrue(
            try row(in: state, id: "thread-10").presentation.isRunning,
            "After clearing higher-priority slots, recent running becomes authoritative."
        )
    }

    func testOptimisticRollbackRestoresExplicitLaterPlacement() {
        let input = GaryxHomeListFixture.makeInputs(threadCount: 10, pinnedCount: 2, runningCount: 0)
        var state = reduce(HomeProjectionState(), ingest(input, epoch: 1)).state

        state = reduce(
            state,
            .optimisticArchive(
                threadId: "thread-0",
                pinnedThreadIds: ["thread-1"],
                recentThreadIds: Array(input.recentThreadIds.dropFirst())
            )
        ).state
        XCTAssertNil(state.snapshot.sections.allRows.first { $0.id == "thread-0" })

        state = reduce(state, .pinsChanged(pinnedThreadIds: ["thread-2"])).state
        state = reduce(
            state,
            .optimisticRollback(
                threadId: "thread-0",
                restoredPinnedThreadIds: ["thread-2", "thread-0"],
                restoredRecentThreadIds: input.recentThreadIds
            )
        ).state

        XCTAssertEqual(state.pinnedThreadIds, ["thread-2", "thread-0"])
        XCTAssertEqual(state.snapshot.sections.pinned.map(\.id), ["thread-2", "thread-0"])
    }

    func testReducerCanRunOffMainActor() async throws {
        let input = GaryxHomeListFixture.makeInputs(threadCount: 30, pinnedCount: 3, runningCount: 0)
        let initialEvent = ingest(input, epoch: 1)

        let output = await Task.detached(priority: .utility) {
            XCTAssertFalse(Thread.isMainThread)
            var state = HomeProjectionState()
            state = HomeProjectionReducer.reduce(state, initialEvent).state
            state = HomeProjectionReducer.reduce(
                state,
                .runStateDelta(
                    source: .runTracker,
                    threadId: "thread-10",
                    status: .running,
                    basedOnSeq: 1
                )
            ).state
            acceptSendableValue(state)
            acceptSendableValue(state.snapshot)
            return state
        }.value

        XCTAssertTrue(try row(in: output, id: "thread-10").presentation.isRunning)
    }

    private func reduce(
        _ state: HomeProjectionState,
        _ event: HomeProjectionEvent
    ) -> HomeProjectionReducer.Result {
        HomeProjectionReducer.reduce(state, event)
    }

    private func ingest(
        _ input: HomeThreadSectionsReference.Inputs,
        epoch: Int
    ) -> HomeProjectionEvent {
        .recentThreadsIngested(
            threads: input.threads,
            recentThreadIds: input.recentThreadIds,
            agents: input.agents,
            automations: input.automations,
            recentRunStateEpoch: epoch
        )
    }

    private func assertCheckpointParity(
        _ state: HomeProjectionState,
        _ store: GaryxHomeThreadListStore,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        _ = store.apply(state.legacyCheckpointInput())
        XCTAssertEqual(store.snapshot.sections, state.snapshot.sections, file: file, line: line)
        XCTAssertEqual(store.snapshot.isLoadingThreads, state.snapshot.isLoadingThreads, file: file, line: line)
        XCTAssertEqual(store.snapshot.isHomeVisible, state.snapshot.isHomeVisible, file: file, line: line)
    }

    private func row(in state: HomeProjectionState, id: String) throws -> GaryxHomeThreadRow {
        try XCTUnwrap(state.snapshot.sections.allRows.first { $0.id == id })
    }

}

private func acceptSendableValue<T: Sendable>(_ value: T) {}
