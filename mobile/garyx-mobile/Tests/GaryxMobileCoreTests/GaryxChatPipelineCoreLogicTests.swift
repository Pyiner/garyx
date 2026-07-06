import XCTest
@testable import GaryxMobileCore

// TASK-1751 Core-logic unit tests for the pure pieces of the four structural
// fixes (P1 restore policy, P2 turn-rows cache, P3 window planner, P4 residency
// tracker). The quantified before/after reproductions live in
// GaryxChatPipelineReproTests.

// MARK: - P2 turn-rows cache

final class GaryxTurnRowsCacheTests: XCTestCase {
    private func message(_ id: String, _ text: String = "x") -> GaryxMobileMessage {
        GaryxMobileMessage(id: id, role: .user, text: text, isStreaming: false)
    }

    private func row(_ id: String) -> GaryxMobileTurnRow {
        GaryxMobileTurnRow(id: id, userBlock: nil, activityRows: [])
    }

    func testUnchangedInputsReuseCachedRowsWithoutRebuilding() {
        var cache = GaryxTurnRowsCache()
        var builds = 0
        let messages = [message("m1")]
        let build: () -> [GaryxMobileTurnRow] = {
            builds += 1
            return [self.row("r1")]
        }
        let first = cache.rows(threadId: "t", snapshot: nil, messages: messages, transcriptMessages: [], build: build)
        let second = cache.rows(threadId: "t", snapshot: nil, messages: messages, transcriptMessages: [], build: build)
        XCTAssertEqual(first, second)
        XCTAssertEqual(builds, 1, "identical inputs must reuse the memo, not rebuild")
    }

    func testChangedMessagesRebuildOnce() {
        var cache = GaryxTurnRowsCache()
        var builds = 0
        _ = cache.rows(threadId: "t", snapshot: nil, messages: [message("m1")], transcriptMessages: []) {
            builds += 1; return [self.row("r1")]
        }
        _ = cache.rows(threadId: "t", snapshot: nil, messages: [message("m1", "changed")], transcriptMessages: []) {
            builds += 1; return [self.row("r2")]
        }
        XCTAssertEqual(builds, 2, "a changed message list must rebuild exactly once")
    }

    func testThreadSwitchRebuilds() {
        var cache = GaryxTurnRowsCache()
        var builds = 0
        _ = cache.rows(threadId: "a", snapshot: nil, messages: [], transcriptMessages: []) {
            builds += 1; return []
        }
        _ = cache.rows(threadId: "b", snapshot: nil, messages: [], transcriptMessages: []) {
            builds += 1; return []
        }
        XCTAssertEqual(builds, 2)
    }

    func testChangedTranscriptMessagesRebuild() {
        var cache = GaryxTurnRowsCache()
        var builds = 0
        _ = cache.rows(threadId: "t", snapshot: nil, messages: [], transcriptMessages: [
            GaryxTranscriptMessage(index: 0, role: .assistant, text: "a")
        ]) { builds += 1; return [] }
        _ = cache.rows(threadId: "t", snapshot: nil, messages: [], transcriptMessages: [
            GaryxTranscriptMessage(index: 0, role: .assistant, text: "b")
        ]) { builds += 1; return [] }
        XCTAssertEqual(builds, 2, "transcript-message changes are part of the mapper key")
    }

    func testCachedIdsMatchRowsAndAreNilBeforeFirstBuild() {
        var cache = GaryxTurnRowsCache()
        XCTAssertNil(cache.cachedIds, "no ids before the first build")
        let rows = cache.rows(threadId: "t", snapshot: nil, messages: [], transcriptMessages: []) {
            [self.row("r1"), self.row("r2")]
        }
        XCTAssertEqual(cache.cachedIds, rows.map(\.id))
        XCTAssertEqual(cache.cachedIds, ["r1", "r2"])
    }

    func testInvalidateForcesRebuild() {
        var cache = GaryxTurnRowsCache()
        var builds = 0
        _ = cache.rows(threadId: "t", snapshot: nil, messages: [], transcriptMessages: []) { builds += 1; return [] }
        cache.invalidate()
        XCTAssertNil(cache.cachedIds)
        _ = cache.rows(threadId: "t", snapshot: nil, messages: [], transcriptMessages: []) { builds += 1; return [] }
        XCTAssertEqual(builds, 2)
    }
}

// MARK: - P3 floor-anchored window planner

final class GaryxTurnRowsWindowPlannerTests: XCTestCase {
    private func rows(_ count: Int, prefix: String = "r") -> [GaryxMobileTurnRow] {
        (0..<count).map { GaryxMobileTurnRow(id: "\(prefix)\($0)", userBlock: nil, activityRows: []) }
    }

    func testUninitializedResolvesToNewestInitialLimit() {
        let all = rows(3000)
        let (visible, state) = GaryxTurnRowsWindowPlanner.resolve(rows: all, state: .init())
        XCTAssertEqual(visible.count, GaryxTurnRowsWindowPlanner.initialLimit)
        XCTAssertEqual(visible.first?.id, "r\(3000 - GaryxTurnRowsWindowPlanner.initialLimit)")
        XCTAssertEqual(visible.last?.id, "r2999")
        XCTAssertEqual(state.floorRowId, "r\(3000 - GaryxTurnRowsWindowPlanner.initialLimit)")
    }

    func testShortThreadRendersEveryRow() {
        let all = rows(5)
        let (visible, _) = GaryxTurnRowsWindowPlanner.resolve(rows: all, state: .init())
        XCTAssertEqual(visible.count, 5)
        XCTAssertTrue(GaryxTurnRowsWindowPlanner.isWindowExhausted(rows: all, state: .init()))
    }

    /// The anti-regression invariant: a tail append with an initialized floor
    /// still present must NOT change which rows are hidden at the top.
    func testTailAppendDoesNotChangeHiddenHeadSet() {
        let base = rows(200)
        let (_, state) = GaryxTurnRowsWindowPlanner.resolve(rows: base, state: .init())
        let hiddenBefore = Set(base.prefix(base.count - GaryxTurnRowsWindowPlanner.initialLimit).map(\.id))

        // Stream appends 40 new tail rows.
        let appended = base + (200..<240).map { GaryxMobileTurnRow(id: "r\($0)", userBlock: nil, activityRows: []) }
        let (visibleAfter, stateAfter) = GaryxTurnRowsWindowPlanner.resolve(rows: appended, state: state)
        let hiddenAfter = Set(appended.filter { row in !visibleAfter.contains(where: { $0.id == row.id }) }.map(\.id))

        XCTAssertEqual(hiddenBefore, hiddenAfter, "a tail append must not add/remove hidden head rows")
        XCTAssertEqual(stateAfter.floorRowId, state.floorRowId, "the floor anchor must not move on a tail append")
        // Window grew only at the bottom: newest appended row is visible.
        XCTAssertEqual(visibleAfter.last?.id, "r239")
        XCTAssertTrue(visibleAfter.contains(where: { $0.id == "r199" }), "previously-visible rows stay visible")
    }

    func testExpandLowersFloorByStepAndClampsAtZero() {
        let all = rows(200)
        let (_, initial) = GaryxTurnRowsWindowPlanner.resolve(rows: all, state: .init())
        // initial floor index = 200 - 60 = 140
        let expanded = GaryxTurnRowsWindowPlanner.expand(rows: all, state: initial)
        XCTAssertEqual(expanded.floorRowId, "r80", "floor lowers by expandStep (140 - 60)")

        // Expand repeatedly clamps at the list start.
        var s = expanded
        for _ in 0..<10 { s = GaryxTurnRowsWindowPlanner.expand(rows: all, state: s) }
        XCTAssertEqual(s.floorRowId, "r0")
        XCTAssertTrue(GaryxTurnRowsWindowPlanner.isWindowExhausted(rows: all, state: s))
    }

    func testExpandedFloorSurvivesSubsequentTailAppend() {
        let all = rows(200)
        let (_, initial) = GaryxTurnRowsWindowPlanner.resolve(rows: all, state: .init())
        let expanded = GaryxTurnRowsWindowPlanner.expand(rows: all, state: initial) // floor r80
        let appended = all + (200..<210).map { GaryxMobileTurnRow(id: "r\($0)", userBlock: nil, activityRows: []) }
        let (visible, stateAfter) = GaryxTurnRowsWindowPlanner.resolve(rows: appended, state: expanded)
        XCTAssertEqual(stateAfter.floorRowId, "r80", "expansion floor persists across a tail append")
        XCTAssertEqual(visible.first?.id, "r80")
        XCTAssertEqual(visible.last?.id, "r209")
    }

    func testDroppedFloorRowReAnchorsToTail() {
        let all = rows(200)
        // Floor anchored deep in history.
        let state = GaryxTurnRowsWindowState(floorRowId: "r10")
        // A windowed-resume reset drops rows below index 150.
        let reset = Array(all[150...])
        let (visible, stateAfter) = GaryxTurnRowsWindowPlanner.resolve(rows: reset, state: state)
        // r10 gone -> re-anchor to newest initialLimit of the 50 remaining -> all 50.
        XCTAssertEqual(visible.count, 50)
        XCTAssertEqual(stateAfter.floorRowId, "r150")
    }

    func testExhaustionFalseWhenHeadRowsHidden() {
        let all = rows(200)
        let (_, state) = GaryxTurnRowsWindowPlanner.resolve(rows: all, state: .init())
        XCTAssertFalse(GaryxTurnRowsWindowPlanner.isWindowExhausted(rows: all, state: state))
    }

    /// Expansion must produce exactly the prepend shape the shipped scroll
    /// policy recognizes (previous first id moved down), so it reuses the
    /// position-preserving path with no new scroll event category.
    func testExpansionProducesRecognizedPrependShape() {
        let all = rows(200)
        let (before, initial) = GaryxTurnRowsWindowPlanner.resolve(rows: all, state: .init())
        let expandedState = GaryxTurnRowsWindowPlanner.expand(rows: all, state: initial)
        let (after, _) = GaryxTurnRowsWindowPlanner.resolve(rows: all, state: expandedState)
        XCTAssertTrue(
            GaryxConversationScrollState.preservesScrollForPrependedHistory(
                previousIds: before.map(\.id),
                currentIds: after.map(\.id),
                threadUnchanged: true
            ),
            "window expansion must look like a recognized history prepend"
        )
    }

    func testEmptyRowsClearFloor() {
        let (visible, state) = GaryxTurnRowsWindowPlanner.resolve(rows: [], state: .init(floorRowId: "r5"))
        XCTAssertTrue(visible.isEmpty)
        XCTAssertNil(state.floorRowId)
    }
}

// MARK: - P4 residency tracker

final class GaryxThreadResidencyTrackerTests: XCTestCase {
    func testEvictsOverCapLeastRecentlyUsed() {
        var tracker = GaryxThreadResidencyTracker(maxResidentThreads: 3)
        for id in ["a", "b", "c", "d", "e"] { tracker.touch(id) }
        let evicted = tracker.evict(pinned: [])
        XCTAssertEqual(evicted, ["a", "b"], "oldest two over the cap of 3")
        XCTAssertEqual(tracker.residentThreadIds, ["c", "d", "e"])
    }

    func testTouchRefreshesRecency() {
        var tracker = GaryxThreadResidencyTracker(maxResidentThreads: 3)
        for id in ["a", "b", "c"] { tracker.touch(id) }
        tracker.touch("a") // a is now most-recent
        tracker.touch("d") // over cap
        let evicted = tracker.evict(pinned: [])
        XCTAssertEqual(evicted, ["b"], "b is now the least-recently-used, not a")
        XCTAssertEqual(tracker.residentThreadIds, ["c", "a", "d"])
    }

    func testPinnedThreadsAreNeverEvictedNorCounted() {
        var tracker = GaryxThreadResidencyTracker(maxResidentThreads: 2)
        for id in ["a", "b", "c", "d"] { tracker.touch(id) }
        // Pin the two oldest; only non-pinned count against the cap of 2.
        let evicted = tracker.evict(pinned: ["a", "b"])
        XCTAssertTrue(evicted.isEmpty, "2 non-pinned (c,d) == cap, nothing evicts")
        XCTAssertEqual(Set(tracker.residentThreadIds), Set(["a", "b", "c", "d"]))
    }

    func testPinnedExemptionWithOverflow() {
        var tracker = GaryxThreadResidencyTracker(maxResidentThreads: 1)
        for id in ["a", "b", "c", "d"] { tracker.touch(id) }
        let evicted = tracker.evict(pinned: ["d"]) // d pinned; a,b,c non-pinned, cap 1
        XCTAssertEqual(evicted, ["a", "b"], "evict oldest non-pinned down to cap 1 (keep c)")
        XCTAssertEqual(tracker.residentThreadIds, ["c", "d"])
    }

    func testRemoveAndBlankIdsIgnored() {
        var tracker = GaryxThreadResidencyTracker(maxResidentThreads: 5)
        tracker.touch("a")
        tracker.touch("   ") // blank ignored
        tracker.touch("")
        XCTAssertEqual(tracker.residentThreadIds, ["a"])
        tracker.remove("a")
        XCTAssertTrue(tracker.residentThreadIds.isEmpty)
    }

    func testManyThreadsBoundedToCapPlusPinned() {
        var tracker = GaryxThreadResidencyTracker(maxResidentThreads: 6)
        for i in 0..<40 { tracker.touch("thread::\(i)") }
        _ = tracker.evict(pinned: ["thread::39"])
        XCTAssertLessThanOrEqual(tracker.count, 6 + 1, "40 visited threads bounded to cap + pinned")
        XCTAssertTrue(tracker.residentThreadIds.contains("thread::39"))
    }
}

// MARK: - P4 unsettled-local-rows pin predicate

final class GaryxThreadResidencyPolicyTests: XCTestCase {
    private func message(_ id: String, role: GaryxMobileMessage.Role, local: GaryxTranscriptEntryState?) -> GaryxMobileMessage {
        GaryxMobileMessage(id: id, role: role, text: "t", isStreaming: false, localState: local)
    }

    func testRemoteFinalOnlyRowsAreSettled() {
        let messages = [
            message("a", role: .user, local: .remoteFinal),
            message("b", role: .assistant, local: .remoteFinal),
            message("c", role: .assistant, local: nil), // synthetic fixture, no local state
        ]
        XCTAssertFalse(GaryxThreadResidencyPolicy.hasUnsettledLocalRows(messages),
                       "remote-final / no-local-state rows are durable — safe to evict")
    }

    func testOptimisticRowIsUnsettled() {
        let messages = [
            message("a", role: .user, local: .remoteFinal),
            message("pending", role: .user, local: .optimistic),
        ]
        XCTAssertTrue(GaryxThreadResidencyPolicy.hasUnsettledLocalRows(messages),
                      "an optimistic (not-yet-acked) send must pin the thread against eviction")
    }

    func testRemotePartialRowIsUnsettled() {
        let messages = [message("streaming", role: .assistant, local: .remotePartial)]
        XCTAssertTrue(GaryxThreadResidencyPolicy.hasUnsettledLocalRows(messages),
                      "a streaming/pending partial row is not durable yet")
    }

    func testEmptyIsSettled() {
        XCTAssertFalse(GaryxThreadResidencyPolicy.hasUnsettledLocalRows([]))
    }
}

// MARK: - P1 cold-open restore policy

final class GaryxColdOpenRestorePolicyTests: XCTestCase {
    private func baseState() -> GaryxColdOpenRestorePolicy.State {
        GaryxColdOpenRestorePolicy.State(
            restoredThreadId: "t",
            selectedThreadId: "t",
            capturedGeneration: 1,
            currentGeneration: 1,
            capturedMirrorGeneration: 5,
            currentMirrorGeneration: 5,
            threadHistoryLoaded: false,
            hasRenderSnapshot: false,
            hasMessages: false
        )
    }

    func testAppliesWhenAllClear() {
        XCTAssertTrue(GaryxColdOpenRestorePolicy.shouldApply(baseState()))
    }

    func testDiscardsWhenThreadChanged() {
        var s = baseState(); s.selectedThreadId = "other"
        XCTAssertFalse(GaryxColdOpenRestorePolicy.shouldApply(s))
    }

    func testDiscardsWhenGenerationBumped() {
        // Switched away and back to the same id: generation advanced.
        var s = baseState(); s.currentGeneration = 2
        XCTAssertFalse(GaryxColdOpenRestorePolicy.shouldApply(s))
    }

    func testDiscardsWhenMirrorGenerationBumped() {
        // The design-review v2 finding-1 gap: a stream `.applyCommittedMessages`
        // wrote the transcript mirror (bumping its generation) between spawn and
        // apply, touching nothing else. Restore must abort so it cannot clobber
        // the fresh committed rows with the stale disk window.
        var s = baseState(); s.currentMirrorGeneration = 6
        XCTAssertFalse(GaryxColdOpenRestorePolicy.shouldApply(s))
        XCTAssertFalse(GaryxColdOpenRestorePolicy.shouldSeedMirror(s),
                       "mirror seed must also abort when a live path advanced the mirror")
    }

    func testDiscardsWhenHistoryLoadedEvenIfEmpty() {
        // The finding-2 arm: a history fetch completed with an empty transcript
        // but still marked history loaded.
        var s = baseState(); s.threadHistoryLoaded = true
        XCTAssertFalse(GaryxColdOpenRestorePolicy.shouldApply(s))
    }

    func testDiscardsWhenRenderSnapshotPresent() {
        // The other finding-2 arm: a stream frame applied a render snapshot
        // before its message flush.
        var s = baseState(); s.hasRenderSnapshot = true
        XCTAssertFalse(GaryxColdOpenRestorePolicy.shouldApply(s))
    }

    func testDiscardsWhenMessagesPresent() {
        var s = baseState(); s.hasMessages = true
        XCTAssertFalse(GaryxColdOpenRestorePolicy.shouldApply(s))
    }

    func testMirrorSeedIsLooserThanApplyButStillGuarded() {
        // Mirror seeding tolerates loaded history + present messages (it only
        // advances the cursor), but must still refuse on thread change, either
        // generation bump, or a live render snapshot.
        var s = baseState(); s.threadHistoryLoaded = true; s.hasMessages = true
        XCTAssertTrue(GaryxColdOpenRestorePolicy.shouldSeedMirror(s))

        var changed = s; changed.selectedThreadId = "other"
        XCTAssertFalse(GaryxColdOpenRestorePolicy.shouldSeedMirror(changed))

        var bumped = s; bumped.currentGeneration = 9
        XCTAssertFalse(GaryxColdOpenRestorePolicy.shouldSeedMirror(bumped))

        var mirrorBumped = s; mirrorBumped.currentMirrorGeneration = 99
        XCTAssertFalse(GaryxColdOpenRestorePolicy.shouldSeedMirror(mirrorBumped))

        var snapshot = s; snapshot.hasRenderSnapshot = true
        XCTAssertFalse(GaryxColdOpenRestorePolicy.shouldSeedMirror(snapshot))
    }
}
