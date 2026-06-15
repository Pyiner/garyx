import XCTest
@testable import GaryxMobileCore

/// UI-free coverage of the mobile "message pulling" decisions: which action a
/// history page drives (overwrite / shrink-refetch / forward-merge), how a streamed
/// committed seq is handled (gap-reconnect / stale-skip / apply), and the stream
/// resume cursor. No simulator, no app target.
final class GaryxTranscriptSyncPlannerTests: XCTestCase {

    // MARK: - Fetch page action

    func testPageActionResetTakesPrecedence() {
        // `reset` wins even when the cursor also looks far past the total.
        XCTAssertEqual(
            GaryxTranscriptFetchPlanner.pageAction(
                cursor: 999, reset: true, hasMoreAfter: true, totalMessagesInThread: 3),
            .reset
        )
    }

    func testPageActionShrinkWhenCursorAtOrBeyondTotal() {
        XCTAssertEqual(
            GaryxTranscriptFetchPlanner.pageAction(
                cursor: 150, reset: false, hasMoreAfter: false, totalMessagesInThread: 150),
            .shrinkRefetch
        )
        XCTAssertEqual(
            GaryxTranscriptFetchPlanner.pageAction(
                cursor: 200, reset: false, hasMoreAfter: false, totalMessagesInThread: 150),
            .shrinkRefetch
        )
    }

    func testPageActionInSyncCursorMergesNotShrinks() {
        // cursor == total - 1 is the max cached index when fully in sync: a merge, not
        // a shrink.
        XCTAssertEqual(
            GaryxTranscriptFetchPlanner.pageAction(
                cursor: 149, reset: false, hasMoreAfter: false, totalMessagesInThread: 150),
            .mergeForward(committedOnly: false, continuePaging: false)
        )
    }

    func testPageActionMergeForwardWithMorePages() {
        XCTAssertEqual(
            GaryxTranscriptFetchPlanner.pageAction(
                cursor: 10, reset: false, hasMoreAfter: true, totalMessagesInThread: 500),
            .mergeForward(committedOnly: true, continuePaging: true)
        )
    }

    func testPageActionMergeForwardFinalPage() {
        XCTAssertEqual(
            GaryxTranscriptFetchPlanner.pageAction(
                cursor: 10, reset: false, hasMoreAfter: false, totalMessagesInThread: 50),
            .mergeForward(committedOnly: false, continuePaging: false)
        )
    }

    func testPageActionMergeForwardWhenTotalUnknown() {
        // No total → cannot detect a shrink; merge forward.
        XCTAssertEqual(
            GaryxTranscriptFetchPlanner.pageAction(
                cursor: 999, reset: false, hasMoreAfter: false, totalMessagesInThread: nil),
            .mergeForward(committedOnly: false, continuePaging: false)
        )
    }

    // MARK: - Stream seq decision

    func testStreamSeqFirstRowApplies() {
        // No prior row this connection (0): even a high seq applies, because a reset
        // replay can legitimately start above the cache cursor.
        XCTAssertEqual(GaryxStreamSeqPlanner.decide(incomingSeq: 100, connectionLastSeq: 0), .apply)
        XCTAssertEqual(GaryxStreamSeqPlanner.decide(incomingSeq: 1, connectionLastSeq: 0), .apply)
    }

    func testStreamSeqContiguousApplies() {
        XCTAssertEqual(GaryxStreamSeqPlanner.decide(incomingSeq: 6, connectionLastSeq: 5), .apply)
    }

    func testStreamSeqGapReconnectsFromLastContiguous() {
        XCTAssertEqual(
            GaryxStreamSeqPlanner.decide(incomingSeq: 8, connectionLastSeq: 5),
            .gapReconnect(resumeAfterSeq: 5)
        )
        XCTAssertEqual(
            GaryxStreamSeqPlanner.decide(incomingSeq: 7, connectionLastSeq: 5),
            .gapReconnect(resumeAfterSeq: 5)
        )
    }

    func testStreamSeqStaleSkips() {
        XCTAssertEqual(GaryxStreamSeqPlanner.decide(incomingSeq: 5, connectionLastSeq: 5), .stale)
        XCTAssertEqual(GaryxStreamSeqPlanner.decide(incomingSeq: 3, connectionLastSeq: 5), .stale)
    }

    // MARK: - Resume cursor

    func testResumeCursorFromAfterCursor() {
        XCTAssertEqual(GaryxStreamSeqPlanner.resumeCursor(afterCursor: 10, fallbackMaxIndex: nil), 11)
        XCTAssertEqual(GaryxStreamSeqPlanner.resumeCursor(afterCursor: 0, fallbackMaxIndex: nil), 1)
    }

    func testResumeCursorPrefersAfterCursorOverFallback() {
        XCTAssertEqual(GaryxStreamSeqPlanner.resumeCursor(afterCursor: 5, fallbackMaxIndex: 100), 6)
    }

    func testResumeCursorFallsBackToMaxIndex() {
        XCTAssertEqual(GaryxStreamSeqPlanner.resumeCursor(afterCursor: nil, fallbackMaxIndex: 20), 21)
    }

    func testResumeCursorZeroWhenNothingCached() {
        XCTAssertEqual(GaryxStreamSeqPlanner.resumeCursor(afterCursor: nil, fallbackMaxIndex: nil), 0)
    }
}
