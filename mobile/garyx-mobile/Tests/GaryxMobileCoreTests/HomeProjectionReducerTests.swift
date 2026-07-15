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

            state = reduce(
                state,
                ingest(corpus, epoch: 2, selectedRecentFilter: .nonTask)
            ).state
            assertCheckpointParity(state, store, file: #filePath, line: #line)

            state = reduce(
                state,
                ingest(
                    corpus,
                    epoch: 3,
                    selectedRecentFilter: .nonTask,
                    recentFeedPresentation: .init(
                        isPrimed: false,
                        isRefreshingHead: false,
                        headFailure: true,
                        footerState: .hidden
                    )
                )
            ).state
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

    func testPinSectionChangeReportsOneAssociatedMoveInsteadOfDeleteAndInsert() throws {
        let input = GaryxHomeListFixture.makeInputs(threadCount: 8, pinnedCount: 0, runningCount: 0)
        let initial = reduce(HomeProjectionState(), ingest(input, epoch: 1)).state

        let result = reduce(initial, .pinsChanged(pinnedThreadIds: ["thread-4"]))
        let difference = try XCTUnwrap(result.difference)

        var removalAssociation: Int?
        var insertionAssociation: Int?
        for change in difference {
            switch change {
            case let .remove(_, element, associatedWith):
                if element == "thread-4" {
                    removalAssociation = associatedWith
                }
            case let .insert(_, element, associatedWith):
                if element == "thread-4" {
                    insertionAssociation = associatedWith
                }
            }
        }

        XCTAssertNotNil(
            removalAssociation,
            "A pin transition must describe the row relocation as a move, not a disappearance."
        )
        XCTAssertNotNil(
            insertionAssociation,
            "A pin transition must describe the row relocation as a move, not a second insertion."
        )
    }

    func testPendingPinMovesOneStableListItemAndShieldsAgainstStaleBase() throws {
        let input = GaryxHomeListFixture.makeInputs(threadCount: 8, pinnedCount: 0, runningCount: 0)
        let base = reduce(HomeProjectionState(), ingest(input, epoch: 1)).state.snapshot.sections
        var transitions = GaryxHomeThreadTransitionState()

        XCTAssertTrue(transitions.beginPin(
            threadId: "thread-4",
            pinned: true,
            originalPinned: false,
            recentIndex: 4
        ))
        XCTAssertFalse(transitions.beginPin(
            threadId: "thread-4",
            pinned: false,
            originalPinned: true,
            recentIndex: 4
        ))
        let presented = transitions.presentedSections(from: base)

        XCTAssertEqual(presented.pinned.map(\.id), ["thread-4"])
        XCTAssertEqual(transitions.motion(for: "thread-4"), .pinning)

        let baseItemId = try XCTUnwrap(
            GaryxHomeThreadListLayout.primaryItems(for: snapshot(sections: base))
                .first(where: { item in
                    if case let .thread(row, _) = item { return row.id == "thread-4" }
                    return false
                })
        ).id
        let movedItemId = try XCTUnwrap(
            GaryxHomeThreadListLayout.primaryItems(for: snapshot(sections: presented))
                .first(where: { item in
                    if case let .thread(row, _) = item { return row.id == "thread-4" }
                    return false
                })
        ).id
        XCTAssertEqual(baseItemId, movedItemId)

        transitions.resolvePin(threadId: "thread-4", pinned: true)
        transitions.reconcile(with: base)
        XCTAssertEqual(
            transitions.presentedSections(from: base).pinned.map(\.id),
            ["thread-4"],
            "An old pins refresh must not bounce the row back to Recent."
        )

        let confirmed = reduce(
            reduce(HomeProjectionState(), ingest(input, epoch: 1)).state,
            .pinsChanged(pinnedThreadIds: ["thread-4"])
        ).state.snapshot.sections
        transitions.reconcile(with: confirmed)
        XCTAssertEqual(transitions.motion(for: "thread-4"), .stable)
        XCTAssertEqual(transitions.presentedSections(from: confirmed), confirmed)
    }

    func testArchiveTransitionKeepsPhysicalRowUntilCommitAndRestoresOnFailure() {
        let input = GaryxHomeListFixture.makeInputs(threadCount: 6, pinnedCount: 1, runningCount: 0)
        let base = reduce(HomeProjectionState(), ingest(input, epoch: 1)).state.snapshot.sections
        var transitions = GaryxHomeThreadTransitionState()

        XCTAssertTrue(transitions.beginArchive(threadId: "thread-0"))
        XCTAssertEqual(transitions.motion(for: "thread-0"), .archiving)
        XCTAssertEqual(
            transitions.presentedSections(from: base).allRows.map(\.id),
            base.allRows.map(\.id),
            "Optimistic archive feedback must not change UICollectionView item counts."
        )

        transitions.cancelArchive(threadId: "thread-0")
        XCTAssertEqual(transitions.motion(for: "thread-0"), .stable)

        XCTAssertTrue(transitions.beginArchive(threadId: "thread-0"))
        transitions.commitArchive(threadId: "thread-0")
        transitions.reconcile(with: base)
        XCTAssertEqual(transitions.motion(for: "thread-0"), .archiving)

        let committed = GaryxHomeThreadSections(
            pinned: base.pinned.filter { $0.id != "thread-0" },
            recent: base.recent.filter { $0.id != "thread-0" }
        )
        transitions.reconcile(with: committed)
        XCTAssertEqual(transitions.motion(for: "thread-0"), .stable)
    }

    func testPinFailureMovesBackImmediatelyWithoutWaitingForCanonicalRollback() {
        let input = GaryxHomeListFixture.makeInputs(threadCount: 8, pinnedCount: 0, runningCount: 0)
        let baseState = reduce(HomeProjectionState(), ingest(input, epoch: 1)).state
        let original = baseState.snapshot.sections
        let optimisticBase = reduce(
            baseState,
            .pinsChanged(pinnedThreadIds: ["thread-4"])
        ).state.snapshot.sections
        var transitions = GaryxHomeThreadTransitionState()

        XCTAssertTrue(transitions.beginPin(
            threadId: "thread-4",
            pinned: true,
            originalPinned: false,
            recentIndex: 4
        ))
        transitions.rollbackPin(threadId: "thread-4")
        transitions.reconcile(with: optimisticBase)

        XCTAssertFalse(transitions.presentedSections(from: optimisticBase).pinned.contains { $0.id == "thread-4" })
        XCTAssertEqual(
            transitions.presentedSections(from: optimisticBase).recent.map(\.id),
            original.recent.map(\.id)
        )

        transitions.reconcile(with: original)
        XCTAssertEqual(transitions.motion(for: "thread-4"), .stable)
    }

    func testConcurrentPinIntentMergePreservesIndependentRequestsAndRollbacks() {
        var transitions = GaryxHomeThreadTransitionState()
        XCTAssertTrue(transitions.beginPin(
            threadId: "thread-a",
            pinned: true,
            originalPinned: false,
            recentIndex: 0
        ))
        XCTAssertTrue(transitions.beginPin(
            threadId: "thread-b",
            pinned: true,
            originalPinned: false,
            recentIndex: 1
        ))
        XCTAssertEqual(
            transitions.presentedPinnedThreadIds(from: []),
            ["thread-b", "thread-a"]
        )

        transitions.rollbackPin(threadId: "thread-a")
        XCTAssertEqual(
            transitions.presentedPinnedThreadIds(from: ["thread-a"]),
            ["thread-b"],
            "rolling back A must retain B while removing A from an older full-list base"
        )

        transitions.rollbackPin(threadId: "thread-b")
        XCTAssertEqual(
            transitions.presentedPinnedThreadIds(from: ["thread-b"]),
            [],
            "rolling back B must not resurrect the already failed A request"
        )
    }

    func testConcurrentPinThenUnpinFailuresRestoreStableNeighborOrder() throws {
        let input = GaryxHomeListFixture.makeInputs(threadCount: 8, pinnedCount: 3, runningCount: 0)
        let ingested = reduce(HomeProjectionState(), ingest(input, epoch: 1)).state
        let sections = reduce(
            ingested,
            .pinsChanged(pinnedThreadIds: input.pinnedThreadIds)
        ).state.snapshot.sections
        let store = GaryxHomeThreadListStore(snapshot: snapshot(sections: sections))

        XCTAssertTrue(store.beginPinTransition(
            threadId: "thread-3",
            pinned: true,
            originalPinned: false,
            recentIndex: 0
        ))
        XCTAssertEqual(
            store.presentationSnapshot.sections.pinned.map(\.id),
            ["thread-3", "thread-0", "thread-1", "thread-2"]
        )
        XCTAssertTrue(store.beginPinTransition(
            threadId: "thread-1",
            pinned: false,
            originalPinned: true,
            recentIndex: 0
        ))

        let afterFirstFailure = try XCTUnwrap(store.rollbackPinTransition(
            threadId: "thread-3",
            basePinnedIds: ["thread-3", "thread-0", "thread-2"]
        ))
        XCTAssertEqual(afterFirstFailure, ["thread-0", "thread-2"])

        let afterSecondFailure = try XCTUnwrap(store.rollbackPinTransition(
            threadId: "thread-1",
            basePinnedIds: afterFirstFailure
        ))
        XCTAssertEqual(
            afterSecondFailure,
            ["thread-0", "thread-1", "thread-2"],
            "A rollback must use surviving stable neighbors, not a stale optimistic array index."
        )
    }

    func testFastPinFailureDerivesRollbackBeforeTransitionReconciliation() throws {
        let input = GaryxHomeListFixture.makeInputs(threadCount: 4, pinnedCount: 0, runningCount: 0)
        let sections = reduce(HomeProjectionState(), ingest(input, epoch: 1)).state.snapshot.sections
        let store = GaryxHomeThreadListStore(snapshot: snapshot(sections: sections))
        XCTAssertTrue(store.beginPinTransition(
            threadId: "thread-1",
            pinned: true,
            originalPinned: false,
            recentIndex: 1
        ))

        let rollbackIds = try XCTUnwrap(
            store.rollbackPinTransition(
                threadId: "thread-1",
                basePinnedIds: ["thread-1"]
            )
        )

        XCTAssertEqual(rollbackIds, [])
        XCTAssertEqual(store.rowMotion(threadId: "thread-1"), .stable)
    }

    func testUnpinOutsideCurrentFilterCollapsesInPlaceAndCanRestoreFromSavedRow() throws {
        let input = GaryxHomeListFixture.makeInputs(threadCount: 8, pinnedCount: 3, runningCount: 0)
        let ingested = reduce(HomeProjectionState(), ingest(input, epoch: 1)).state
        let base = reduce(
            ingested,
            .pinsChanged(pinnedThreadIds: input.pinnedThreadIds)
        ).state.snapshot.sections
        let pinnedRow = try XCTUnwrap(base.pinned.first { $0.id == "thread-1" })
        let originalPinnedIndex = try XCTUnwrap(base.pinned.firstIndex { $0.id == pinnedRow.id })
        var transitions = GaryxHomeThreadTransitionState()

        XCTAssertTrue(transitions.beginPin(
            threadId: pinnedRow.id,
            pinned: false,
            originalPinned: true,
            recentIndex: nil,
            originalPinnedIndex: originalPinnedIndex,
            originalPinnedOrder: base.pinned.map(\.id),
            originalRecentOrder: base.recent.map(\.id),
            sourceRow: pinnedRow
        ))
        XCTAssertEqual(transitions.motion(for: pinnedRow.id), .leavingFilteredList)
        XCTAssertEqual(
            transitions.presentedSections(from: base).pinned.map(\.id),
            base.pinned.map(\.id),
            "The physical List item stays in its source slot while it visually collapses."
        )

        let optimisticBase = GaryxHomeThreadSections(
            pinned: base.pinned.filter { $0.id != pinnedRow.id },
            recent: base.recent
        )
        transitions.rollbackPin(threadId: pinnedRow.id)
        transitions.reconcile(with: optimisticBase)
        XCTAssertEqual(
            transitions.presentedSections(from: optimisticBase).pinned.map(\.id),
            base.pinned.map(\.id),
            "A failed request restores the saved source row before canonical rollback catches up."
        )

        transitions.reconcile(with: base)
        XCTAssertEqual(transitions.motion(for: pinnedRow.id), .stable)
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
        epoch: Int,
        selectedRecentFilter: GaryxRecentThreadFilter = .all,
        recentFeedPresentation: GaryxRecentThreadFeedPresentation = .init(isPrimed: true)
    ) -> HomeProjectionEvent {
        .recentThreadsIngested(
            threads: input.threads,
            recentThreadIds: input.recentThreadIds,
            agents: input.agents,
            automations: input.automations,
            selectedRecentFilter: selectedRecentFilter,
            recentFeedPresentation: recentFeedPresentation,
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
        XCTAssertEqual(store.snapshot.selectedRecentFilter, state.snapshot.selectedRecentFilter, file: file, line: line)
        XCTAssertEqual(store.snapshot.recentFeedPresentation, state.snapshot.recentFeedPresentation, file: file, line: line)
    }

    private func row(in state: HomeProjectionState, id: String) throws -> GaryxHomeThreadRow {
        try XCTUnwrap(state.snapshot.sections.allRows.first { $0.id == id })
    }

    private func snapshot(sections: GaryxHomeThreadSections) -> GaryxHomeThreadListSnapshot {
        GaryxHomeThreadListSnapshot(
            sections: sections,
            isLoadingThreads: false,
            isHomeVisible: true,
            selectedRecentFilter: .all,
            recentFeedPresentation: .init(isPrimed: true)
        )
    }

}

private func acceptSendableValue<T: Sendable>(_ value: T) {}
