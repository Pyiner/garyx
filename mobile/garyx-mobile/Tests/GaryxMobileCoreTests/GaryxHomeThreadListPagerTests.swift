import XCTest
@testable import GaryxMobileCore

/// TASK-1802: home recent-thread list paging state machine. Test-plan
/// numbering follows docs/design/task-1802-ios-home-list-refresh-loadmore.md.
final class GaryxHomeThreadListPagerTests: XCTestCase {
    private func makePager(pageLimit: Int = 30, overlap: Int = 5) -> GaryxHomeThreadListPager {
        GaryxHomeThreadListPager(pageLimit: pageLimit, overlap: overlap)
    }

    /// Primes the pager with a first head page ending at `cursor`.
    private func primedPager(
        pageLimit: Int = 30,
        overlap: Int = 5,
        cursor: Int = 30,
        hasMore: Bool = true
    ) -> GaryxHomeThreadListPager {
        var pager = makePager(pageLimit: pageLimit, overlap: overlap)
        let ticket = pager.requestRefresh()!
        pager.completeRefresh(ticket, pageOffset: 0, pageCount: cursor, hasMore: hasMore)
        return pager
    }

    private func stateTuple(
        _ pager: GaryxHomeThreadListPager
    ) -> (gate: GaryxHomeThreadListPager.LoadMoreGate, revision: Int, cursor: Int, hasMore: Bool) {
        (pager.gate, pager.loadMoreFailureRevision, pager.nextOffset, pager.hasMoreThreadSummaries)
    }

    private func assertTuplesEqual(
        _ lhs: GaryxHomeThreadListPager,
        _ rhs: GaryxHomeThreadListPager,
        _ message: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let a = stateTuple(lhs)
        let b = stateTuple(rhs)
        XCTAssertEqual(a.gate, b.gate, "gate — \(message)", file: file, line: line)
        XCTAssertEqual(a.revision, b.revision, "revision — \(message)", file: file, line: line)
        XCTAssertEqual(a.cursor, b.cursor, "cursor — \(message)", file: file, line: line)
        XCTAssertEqual(a.hasMore, b.hasMore, "hasMore — \(message)", file: file, line: line)
    }

    // MARK: 1. Initial refresh primes cursor and gate

    func testInitialRefreshPrimesCursorFromDecodedPage() throws {
        // Real cursor-only gateway response shape. The legacy offset pager is
        // still exercised here as an isolated gate primitive until the Recent
        // range-fill state replaces it.
        let json = Data("""
        {
          "threads": [
            {"thread_id": "thread::11111111-aaaa-bbbb-cccc-000000000001",
             "title": "Fix login flow", "last_active_at": "2026-07-07T02:00:00Z",
             "last_message_preview": "done, tests green", "agent_id": "claude",
             "activity_seq": 3},
            {"thread_id": "thread::11111111-aaaa-bbbb-cccc-000000000002",
             "title": "Design review", "last_active_at": "2026-07-07T01:00:00Z",
             "last_message_preview": "PASS", "agent_id": "codex", "activity_seq": 2},
            {"thread_id": "thread::11111111-aaaa-bbbb-cccc-000000000003",
             "title": "Untitled", "last_active_at": "2026-07-07T00:30:00Z",
             "last_message_preview": "", "activity_seq": 1}
          ],
          "count": 3, "limit": 3, "total": 12, "has_more": true,
          "next_cursor": "cursor-1",
          "store_incarnation_id": "11111111-1111-4111-8111-111111111111",
          "server_boot_id": "22222222-2222-4222-8222-222222222222"
        }
        """.utf8)
        let page = try JSONDecoder().decode(GaryxRecentThreadsPage.self, from: json)
        XCTAssertTrue(page.hasMore)

        var pager = makePager(pageLimit: 3)
        XCTAssertNil(
            pager.requestLoadMore(trigger: .footer),
            "not primed: load-more is meaningless before the first head page"
        )

        let ticket = try XCTUnwrap(pager.requestRefresh())
        let application = pager.completeRefresh(
            ticket,
            pageOffset: 0,
            pageCount: page.count,
            hasMore: page.hasMore
        )
        XCTAssertEqual(application, .apply(.replaceHead))
        XCTAssertEqual(pager.nextOffset, 3)
        XCTAssertEqual(pager.gate, .ready)
        XCTAssertTrue(pager.hasMoreThreadSummaries)
        XCTAssertFalse(pager.isRefreshingHead)
        XCTAssertNotNil(pager.requestLoadMore(trigger: .footer), "primed now")
    }

    // MARK: 2. Load-more happy path

    func testLoadMoreHappyPathUsesOverlapAndServerCursor() throws {
        var pager = primedPager(cursor: 30)
        let ticket = try XCTUnwrap(pager.requestLoadMore(trigger: .nearTail))
        XCTAssertEqual(ticket.offset, 25, "request backs off by overlap")
        XCTAssertEqual(ticket.limit, 30)
        XCTAssertTrue(pager.isLoadingMore)

        // Server answers the overlapped request: offset 25, 30 rows.
        XCTAssertEqual(pager.completeLoadMore(ticket, pageOffset: 25, pageCount: 30, hasMore: true), .apply)
        XCTAssertEqual(pager.nextOffset, 55, "cursor advances from server offset+count, not the raw request")
        XCTAssertEqual(pager.gate, .ready)
        XCTAssertFalse(pager.isLoadingMore)
    }

    func testLoadMoreOffsetFloorsAtZero() throws {
        var pager = primedPager(pageLimit: 3, overlap: 5, cursor: 3)
        let ticket = try XCTUnwrap(pager.requestLoadMore(trigger: .footer))
        XCTAssertEqual(ticket.offset, 0, "overlap larger than cursor floors at 0")
    }

    // MARK: 3. Concurrency ordering matrix

    /// Runs refresh-start → loadMore-start, then completes both in the
    /// given order, returning the final pager.
    private func runBothTracks(
        initial: GaryxHomeThreadListPager,
        refreshResult: (success: Bool, pageOffset: Int, pageCount: Int, hasMore: Bool),
        loadMoreResult: (success: Bool, hasMore: Bool),
        refreshCompletesFirst: Bool
    ) -> GaryxHomeThreadListPager {
        var pager = initial
        let refreshTicket = pager.requestRefresh()!
        let loadMoreTicket = pager.gate == .failed
            ? pager.retryLoadMore()!
            : pager.requestLoadMore(trigger: .nearTail)!

        func finishRefresh() {
            if refreshResult.success {
                pager.completeRefresh(
                    refreshTicket,
                    pageOffset: refreshResult.pageOffset,
                    pageCount: refreshResult.pageCount,
                    hasMore: refreshResult.hasMore
                )
            } else {
                pager.failRefresh(refreshTicket)
            }
        }
        func finishLoadMore() {
            if loadMoreResult.success {
                pager.completeLoadMore(
                    loadMoreTicket,
                    pageOffset: loadMoreTicket.offset,
                    pageCount: loadMoreTicket.limit,
                    hasMore: loadMoreResult.hasMore
                )
            } else {
                pager.failLoadMore(loadMoreTicket)
            }
        }

        if refreshCompletesFirst {
            finishRefresh()
            finishLoadMore()
        } else {
            finishLoadMore()
            finishRefresh()
        }
        XCTAssertFalse(pager.isRefreshingHead)
        XCTAssertFalse(pager.isLoadingMore)
        return pager
    }

    func testOrderingMatrixFinalStateIsCompletionOrderCommutative() {
        let refreshOutcomes: [(success: Bool, pageOffset: Int, pageCount: Int, hasMore: Bool)] = [
            (success: true, pageOffset: 0, pageCount: 30, hasMore: true),
            (success: false, pageOffset: 0, pageCount: 0, hasMore: false),
        ]
        let loadMoreOutcomes: [(success: Bool, hasMore: Bool)] = [
            (success: true, hasMore: true),
            (success: false, hasMore: false),
        ]
        for refreshOutcome in refreshOutcomes {
            for loadMoreOutcome in loadMoreOutcomes {
                let first = runBothTracks(
                    initial: primedPager(cursor: 30),
                    refreshResult: refreshOutcome,
                    loadMoreResult: loadMoreOutcome,
                    refreshCompletesFirst: true
                )
                let second = runBothTracks(
                    initial: primedPager(cursor: 30),
                    refreshResult: refreshOutcome,
                    loadMoreResult: loadMoreOutcome,
                    refreshCompletesFirst: false
                )
                assertTuplesEqual(
                    first,
                    second,
                    "refresh success=\(refreshOutcome.success) loadMore success=\(loadMoreOutcome.success)"
                )
            }
        }
    }

    func testLoadMoreFailureBesideSuccessfulRefreshSurvivesBothOrders() {
        for refreshFirst in [true, false] {
            let pager = runBothTracks(
                initial: primedPager(cursor: 30),
                refreshResult: (success: true, pageOffset: 0, pageCount: 30, hasMore: true),
                loadMoreResult: (success: false, hasMore: false),
                refreshCompletesFirst: refreshFirst
            )
            XCTAssertEqual(
                pager.gate, .failed,
                "a refresh issued before the failure cannot forgive it (refreshFirst=\(refreshFirst))"
            )
            XCTAssertEqual(pager.loadMoreFailureRevision, 1)
        }
    }

    /// Re-review-2 counterexample: gate already .failed when a refresh
    /// and a retry are both issued — both completion orders must end
    /// (.failed, revision + 1).
    func testFailedGateWithConcurrentRefreshAndRetryIsOrderCommutative() {
        func failedPager() -> GaryxHomeThreadListPager {
            var pager = primedPager(cursor: 30)
            let ticket = pager.requestLoadMore(trigger: .footer)!
            pager.failLoadMore(ticket)
            XCTAssertEqual(pager.gate, .failed)
            XCTAssertEqual(pager.loadMoreFailureRevision, 1)
            return pager
        }
        for refreshFirst in [true, false] {
            let pager = runBothTracks(
                initial: failedPager(),
                refreshResult: (success: true, pageOffset: 0, pageCount: 30, hasMore: true),
                loadMoreResult: (success: false, hasMore: false),
                refreshCompletesFirst: refreshFirst
            )
            XCTAssertEqual(pager.gate, .failed, "refreshFirst=\(refreshFirst)")
            XCTAssertEqual(pager.loadMoreFailureRevision, 2, "refreshFirst=\(refreshFirst)")
        }
    }

    // MARK: 4. Self-coalescing, cross-track independence

    func testTracksSelfCoalesceButDoNotBlockEachOther() throws {
        var pager = primedPager(cursor: 30)

        let refreshTicket = try XCTUnwrap(pager.requestRefresh())
        XCTAssertNil(pager.requestRefresh(), "refresh coalesces into the in-flight refresh")

        let loadMoreTicket = try XCTUnwrap(
            pager.requestLoadMore(trigger: .footer),
            "a refresh in flight must not block load-more (the R4 drop scenario)"
        )
        XCTAssertNil(pager.requestLoadMore(trigger: .nearTail), "load-more self-coalesces")

        pager.completeLoadMore(loadMoreTicket, pageOffset: 25, pageCount: 30, hasMore: true)
        XCTAssertNotNil(pager.requestLoadMore(trigger: .nearTail), "granted again after completion")

        pager.completeRefresh(refreshTicket, pageOffset: 0, pageCount: 30, hasMore: true)
        XCTAssertNotNil(pager.requestRefresh(), "granted again after completion")
    }

    func testLoadMoreInFlightDoesNotBlockRefresh() throws {
        var pager = primedPager(cursor: 30)
        _ = try XCTUnwrap(pager.requestLoadMore(trigger: .footer))
        XCTAssertNotNil(pager.requestRefresh(), "a load-more in flight must not block refresh")
    }

    // MARK: 5. Failure ladder

    func testFailureLadderRetryAndRevisionScopedReArm() throws {
        var pager = primedPager(cursor: 30)

        // A refresh issued BEFORE the failure must not forgive it later.
        let staleRefresh = try XCTUnwrap(pager.requestRefresh())

        let ticket = try XCTUnwrap(pager.requestLoadMore(trigger: .nearTail))
        pager.failLoadMore(ticket)
        XCTAssertEqual(pager.gate, .failed)
        XCTAssertEqual(pager.loadMoreFailureRevision, 1)

        XCTAssertNil(pager.requestLoadMore(trigger: .nearTail), "automatic trigger rejected after failure")
        XCTAssertNil(pager.requestLoadMore(trigger: .footer), "automatic trigger rejected after failure")

        pager.completeRefresh(staleRefresh, pageOffset: 0, pageCount: 30, hasMore: true)
        XCTAssertEqual(pager.gate, .failed, "stale observed revision cannot re-arm")

        let retry = try XCTUnwrap(pager.retryLoadMore(), "explicit retry bypasses the failed gate")
        pager.failLoadMore(retry)
        XCTAssertEqual(pager.gate, .failed)
        XCTAssertEqual(pager.loadMoreFailureRevision, 2, "every failure bumps the revision")

        // A refresh issued AFTER the failure re-arms and preserves cursor.
        let freshRefresh = try XCTUnwrap(pager.requestRefresh())
        pager.completeRefresh(freshRefresh, pageOffset: 0, pageCount: 30, hasMore: true)
        XCTAssertEqual(pager.gate, .ready, "refresh issued after the failure forgives it")
        XCTAssertEqual(pager.nextOffset, 30, "cursor preserved")
        XCTAssertNotNil(pager.requestLoadMore(trigger: .nearTail))
    }

    func testRetryRequiresFailedGate() {
        var pager = primedPager(cursor: 30)
        XCTAssertNil(pager.retryLoadMore(), "retry only bypasses the failed gate")
    }

    // MARK: 6. Exhaustion

    func testExhaustionRejectsTriggersAndSurvivesBeyondHeadRefresh() throws {
        var pager = primedPager(cursor: 30)
        let ticket = try XCTUnwrap(pager.requestLoadMore(trigger: .footer))
        pager.completeLoadMore(ticket, pageOffset: 25, pageCount: 30, hasMore: false)
        XCTAssertEqual(pager.gate, .exhausted)
        XCTAssertEqual(pager.nextOffset, 55)
        XCTAssertFalse(pager.hasMoreThreadSummaries)
        XCTAssertEqual(pager.footerState, .hidden)
        XCTAssertNil(pager.requestLoadMore(trigger: .nearTail))
        XCTAssertNil(pager.retryLoadMore(), "exhausted is not retryable")

        // A beyond-head refresh must not resurrect hasMore.
        let refresh = try XCTUnwrap(pager.requestRefresh())
        let application = pager.completeRefresh(refresh, pageOffset: 0, pageCount: 30, hasMore: true)
        XCTAssertEqual(application, .apply(.mergeBeyondHead))
        XCTAssertEqual(pager.gate, .exhausted, "beyond-head refresh keeps pagination untouched")
        XCTAssertEqual(pager.nextOffset, 55)
    }

    // MARK: 7. Reset + stale tickets

    func testResetBumpsEpochAndStaleTicketsAreNoOps() throws {
        var pager = primedPager(cursor: 30)
        let refreshTicket = try XCTUnwrap(pager.requestRefresh())
        let loadMoreTicket = try XCTUnwrap(pager.requestLoadMore(trigger: .footer))

        pager.reset()
        XCTAssertEqual(pager.epoch, 1)
        XCTAssertFalse(pager.isRefreshingHead)
        XCTAssertFalse(pager.isLoadingMore)
        XCTAssertEqual(pager.gate, .ready)
        XCTAssertEqual(pager.nextOffset, 0)
        XCTAssertEqual(pager.loadMoreFailureRevision, 0)

        let afterReset = pager
        XCTAssertEqual(
            pager.completeRefresh(refreshTicket, pageOffset: 0, pageCount: 30, hasMore: true),
            .abandonedStaleEpoch,
            "stale refresh completion tells the caller to drop the page"
        )
        XCTAssertEqual(pager, afterReset, "stale completion is a no-op on every field")

        XCTAssertEqual(
            pager.completeLoadMore(loadMoreTicket, pageOffset: 25, pageCount: 30, hasMore: true),
            .abandonedStaleEpoch
        )
        XCTAssertEqual(pager, afterReset)

        pager.failLoadMore(loadMoreTicket)
        XCTAssertEqual(pager, afterReset, "stale failure cannot poison the new epoch's gate")

        pager.failRefresh(refreshTicket)
        XCTAssertEqual(pager, afterReset)
    }

    // MARK: 8. Cursor semantics (documented micro-change)

    func testHeadRefreshCursorSemantics() throws {
        // Single page loaded: a drifted head page (same numeric shape)
        // replaces the head and adopts the page cursor.
        var singlePage = primedPager(cursor: 30)
        let refresh1 = try XCTUnwrap(singlePage.requestRefresh())
        XCTAssertEqual(
            singlePage.completeRefresh(refresh1, pageOffset: 0, pageCount: 30, hasMore: true),
            .apply(.replaceHead),
            "no load-more yet ⇒ the head page owns the list"
        )
        XCTAssertEqual(singlePage.nextOffset, 30)

        // Multi-page loaded: head refresh merges and keeps pagination.
        var multiPage = primedPager(cursor: 30)
        let loadMore = try XCTUnwrap(multiPage.requestLoadMore(trigger: .footer))
        multiPage.completeLoadMore(loadMore, pageOffset: 25, pageCount: 30, hasMore: true)
        XCTAssertEqual(multiPage.nextOffset, 55)
        let refresh2 = try XCTUnwrap(multiPage.requestRefresh())
        XCTAssertEqual(
            multiPage.completeRefresh(refresh2, pageOffset: 0, pageCount: 30, hasMore: false),
            .apply(.mergeBeyondHead)
        )
        XCTAssertEqual(multiPage.nextOffset, 55, "cursor preserved beyond head")
        XCTAssertEqual(multiPage.gate, .ready, "head page hasMore does not touch pagination beyond head")
    }

    // MARK: 9. mergeHead

    func testMergeHeadNewHeadWinsAndTailKeepsOrder() {
        let existing = ["t1", "t2", "t3", "t4", "t5", "t6"]
        // t9 is new, t3 was promoted into the head, t1/t2 stay.
        let page = ["t9", "t3", "t1", "t2"]
        let merged = GaryxThreadListPageMerge.mergeHead(pageIds: page, existingIds: existing)
        XCTAssertEqual(merged, ["t9", "t3", "t1", "t2", "t4", "t5", "t6"])
        XCTAssertEqual(Set(merged).count, merged.count, "promoted ids are not duplicated in the tail")
    }

    func testMergeHeadEmptyCases() {
        XCTAssertEqual(GaryxThreadListPageMerge.mergeHead(pageIds: [], existingIds: ["a"]), ["a"])
        XCTAssertEqual(GaryxThreadListPageMerge.mergeHead(pageIds: ["a"], existingIds: []), ["a"])
    }

    // MARK: 10. appendPage + drift

    func testAppendPageDedupsOverlapAndHeadInsertDuplicates() {
        let existing = ["t1", "t2", "t3", "t4", "t5"]
        // Overlapped fetch returns two already-seen rows plus new ones.
        let page = ["t4", "t5", "t6", "t7"]
        XCTAssertEqual(
            GaryxThreadListPageMerge.appendPage(pageIds: page, existingIds: existing),
            ["t1", "t2", "t3", "t4", "t5", "t6", "t7"]
        )
    }

    /// Simulates the server-side removal drift from design §4 against a
    /// realistic 100-thread server list.
    private func driftScenario(removedCount: Int, overlap: Int) -> (loaded: [String], skipped: [String]) {
        let server = (1...100).map { "t\($0)" }
        var client = Array(server.prefix(60))     // seen [t1..t60], cursor 60
        var pager = primedPager(overlap: overlap, cursor: 30)
        do { // advance the pager cursor to 60 to match the seen window
            let ticket = pager.requestLoadMore(trigger: .footer)!
            _ = pager.completeLoadMore(ticket, pageOffset: 30, pageCount: 30, hasMore: true)
        }
        // Server deletes `removedCount` rows inside the seen range.
        let removed = Set((10..<(10 + removedCount)).map { "t\($0)" })
        let shifted = server.filter { !removed.contains($0) }
        client.removeAll { removed.contains($0) }

        let ticket = pager.requestLoadMore(trigger: .nearTail)!
        let pageRows = Array(shifted.dropFirst(ticket.offset).prefix(ticket.limit))
        _ = pager.completeLoadMore(
            ticket,
            pageOffset: ticket.offset,
            pageCount: pageRows.count,
            hasMore: true
        )
        let loaded = GaryxThreadListPageMerge.appendPage(pageIds: pageRows, existingIds: client)

        // Rows the user should now have: everything up to the last loaded
        // server position.
        guard let last = loaded.last, let lastIndex = shifted.firstIndex(of: last) else {
            return (loaded, [])
        }
        let expected = Set(shifted.prefix(lastIndex + 1))
        let skipped = shifted.prefix(lastIndex + 1).filter { !Set(loaded).contains($0) }
        XCTAssertEqual(expected.subtracting(loaded).sorted(), skipped.sorted())
        return (loaded, skipped)
    }

    func testOverlapAbsorbsRemovalDriftWithinBudget() {
        let result = driftScenario(removedCount: 5, overlap: 5)
        XCTAssertTrue(result.skipped.isEmpty, "5 removals with overlap 5 skip nothing")
    }

    func testRemovalDriftBeyondOverlapSkipsExactlyTheResidual() {
        // Documented residual (design §4): 6 removals with overlap 5
        // skip exactly one never-seen row.
        let result = driftScenario(removedCount: 6, overlap: 5)
        XCTAssertEqual(result.skipped.count, 1, "the mitigation limit is pinned, not accidental")
    }

    // MARK: 11. prefetchTriggerRowId

    func testPrefetchTriggerRowId() {
        let ids = (1...30).map { "t\($0)" }
        XCTAssertEqual(
            GaryxThreadListPageMerge.prefetchTriggerRowId(recentIds: ids, prefetchDistance: 6),
            "t25",
            "six rows from the end"
        )
        XCTAssertNil(
            GaryxThreadListPageMerge.prefetchTriggerRowId(recentIds: Array(ids.prefix(5)), prefetchDistance: 6),
            "short lists rely on the footer trigger"
        )
        XCTAssertEqual(
            GaryxThreadListPageMerge.prefetchTriggerRowId(recentIds: Array(ids.prefix(6)), prefetchDistance: 6),
            "t1"
        )
        let appended = ids + ["t31", "t32"]
        XCTAssertEqual(
            GaryxThreadListPageMerge.prefetchTriggerRowId(recentIds: appended, prefetchDistance: 6),
            "t27",
            "moves as pages append"
        )
        XCTAssertNil(GaryxThreadListPageMerge.prefetchTriggerRowId(recentIds: ids, prefetchDistance: 0))
    }

    // MARK: 12. Footer state derivation

    func testFooterStateDerivation() throws {
        var pager = makePager()
        XCTAssertEqual(pager.footerState, .hidden, "not primed: nothing to load yet")

        pager = primedPager(cursor: 30)
        XCTAssertEqual(pager.footerState, .idle)

        let loadTicket = try XCTUnwrap(pager.requestLoadMore(trigger: .footer))
        XCTAssertEqual(pager.footerState, .loading)

        pager.failLoadMore(loadTicket)
        XCTAssertEqual(pager.footerState, .failed)

        let retry = try XCTUnwrap(pager.retryLoadMore())
        XCTAssertEqual(pager.footerState, .loading, "retry in flight renders as loading")

        pager.completeLoadMore(retry, pageOffset: 25, pageCount: 30, hasMore: false)
        XCTAssertEqual(pager.footerState, .hidden, "exhausted hides the footer")

        pager = primedPager(cursor: 30, hasMore: false)
        XCTAssertEqual(pager.footerState, .hidden, "single exhausted page hides the footer")

        // A refresh in flight does not affect the footer.
        pager = primedPager(cursor: 30)
        _ = pager.requestRefresh()
        XCTAssertEqual(pager.footerState, .idle)
    }

    // MARK: 14. Local mutation invalidation (review #TASK-1804 round 3)

    /// The reviewer's interleaving: archive succeeds and resolves its
    /// tombstone while a refresh is suspended; commit-point tombstone
    /// filtering can no longer catch it, so the pager must abandon the
    /// stale refresh outright — its surgery marker outlives the tombstone.
    func testLocalMutationAbandonsInFlightRefreshCommit() throws {
        var pager = primedPager(cursor: 30)
        let before = pager
        let ticket = try XCTUnwrap(pager.requestRefresh())

        // Archive (or delete / pin edit) lands while the refresh is out.
        pager.noteLocalMutation()

        XCTAssertEqual(
            pager.completeRefresh(ticket, pageOffset: 0, pageCount: 30, hasMore: false),
            .abandonedLocalMutation,
            "pre-surgery snapshots must not commit even though the gateway call succeeded"
        )
        XCTAssertFalse(pager.isRefreshingHead, "the gate is released for the follow-up")
        XCTAssertEqual(pager.gate, before.gate, "abandonment leaves the gate untouched")
        XCTAssertEqual(pager.nextOffset, before.nextOffset, "abandonment leaves the cursor untouched")
        XCTAssertEqual(pager.loadMoreFailureRevision, before.loadMoreFailureRevision)
        XCTAssertEqual(pager.hasMoreThreadSummaries, before.hasMoreThreadSummaries)

        // The follow-up refresh observes the surgery and commits normally.
        let followUp = try XCTUnwrap(pager.requestRefresh())
        XCTAssertEqual(
            pager.completeRefresh(followUp, pageOffset: 0, pageCount: 30, hasMore: true),
            .apply(.replaceHead)
        )
    }

    func testLocalMutationBeforeIssueDoesNotAbandonRefresh() throws {
        var pager = primedPager(cursor: 30)
        pager.noteLocalMutation()
        let ticket = try XCTUnwrap(pager.requestRefresh())
        XCTAssertEqual(
            pager.completeRefresh(ticket, pageOffset: 0, pageCount: 30, hasMore: true),
            .apply(.replaceHead),
            "surgery that pre-dates the ticket is already reflected in its pages"
        )
    }

    /// Review #TASK-1804 round 4: an overlapped load-more page fetched
    /// before a local removal contains the removed row; dedup-append
    /// against the post-surgery list (which no longer has the id) would
    /// resurrect it as a "new" row. Load-more therefore abandons on local
    /// mutation exactly like refresh.
    func testLocalMutationAbandonsInFlightLoadMore() throws {
        var pager = primedPager(cursor: 30)
        let before = pager
        let ticket = try XCTUnwrap(pager.requestLoadMore(trigger: .footer))

        pager.noteLocalMutation()

        XCTAssertEqual(
            pager.completeLoadMore(ticket, pageOffset: 25, pageCount: 30, hasMore: true),
            .abandonedLocalMutation,
            "a page fetched before the surgery must not be appended"
        )
        XCTAssertFalse(pager.isLoadingMore, "the track is released for the follow-up")
        XCTAssertEqual(pager.nextOffset, before.nextOffset, "the cursor did not advance")
        XCTAssertEqual(pager.gate, before.gate, "abandonment leaves the gate untouched")

        // The follow-up re-requests the same window with a fresh sequence.
        let followUp = try XCTUnwrap(pager.requestLoadMore(trigger: .footer))
        XCTAssertEqual(followUp.offset, ticket.offset, "same window: the cursor never moved")
        XCTAssertEqual(
            pager.completeLoadMore(followUp, pageOffset: 25, pageCount: 30, hasMore: true),
            .apply
        )
        XCTAssertEqual(pager.nextOffset, 55)
    }

    func testLocalMutationBeforeIssueDoesNotAbandonLoadMore() throws {
        var pager = primedPager(cursor: 30)
        pager.noteLocalMutation()
        let ticket = try XCTUnwrap(pager.requestLoadMore(trigger: .footer))
        XCTAssertEqual(
            pager.completeLoadMore(ticket, pageOffset: 25, pageCount: 30, hasMore: true),
            .apply,
            "surgery that pre-dates the ticket is already reflected in its page"
        )
    }

    func testResetClearsLocalMutationSequence() {
        var pager = primedPager(cursor: 30)
        pager.noteLocalMutation()
        pager.noteLocalMutation()
        XCTAssertEqual(pager.localMutationSequence, 2)
        pager.reset()
        XCTAssertEqual(pager.localMutationSequence, 0)
    }

    // MARK: 13. Refresh policy

    func testRefreshFailurePresentationTable() {
        XCTAssertEqual(
            GaryxThreadListRefreshPolicy.failurePresentation(source: .userPullToRefresh),
            .toast
        )
        XCTAssertEqual(
            GaryxThreadListRefreshPolicy.failurePresentation(source: .userAction),
            .toast
        )
        XCTAssertEqual(
            GaryxThreadListRefreshPolicy.failurePresentation(source: .backgroundLoop),
            .transientStatus
        )
    }

    func testShowsSkeletonDerivesFromListEmptiness() {
        XCTAssertTrue(GaryxThreadListRefreshPolicy.showsSkeleton(listIsEmpty: true))
        XCTAssertFalse(GaryxThreadListRefreshPolicy.showsSkeleton(listIsEmpty: false))
    }
}
