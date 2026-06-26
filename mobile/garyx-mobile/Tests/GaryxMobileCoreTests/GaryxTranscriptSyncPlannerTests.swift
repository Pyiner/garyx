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

    func testCommittedStreamBatchWindowIsThreeSeconds() {
        XCTAssertEqual(
            GaryxStreamUpdateCadence.committedMessageBatchWindowNanos,
            3_000_000_000
        )
    }

    // MARK: - Stream seq decision

    func testStreamSeqFirstRowApplies() {
        // No prior row this connection (0): even a high seq applies, because a reset
        // replay can legitimately start above the cache cursor.
        XCTAssertEqual(GaryxStreamSeqPlanner.decide(incomingSeq: 100, connectionLastSeq: 0), .apply)
        XCTAssertEqual(GaryxStreamSeqPlanner.decide(incomingSeq: 7, connectionLastSeq: 0), .apply)
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
        XCTAssertEqual(GaryxStreamSeqPlanner.decide(incomingSeq: 3, connectionLastSeq: 5), .stale)
    }

    func testStreamSeqSameSeqReplacementApplies() {
        XCTAssertEqual(GaryxStreamSeqPlanner.decide(incomingSeq: 5, connectionLastSeq: 5), .apply)
    }

    // MARK: - Rewrite control action

    func testRangeRewriteControlRequiresAuthoritativeRefetch() {
        let old = GaryxTranscriptMessage(index: 2, role: .assistant, text: "old answer")
        let tombstone = controlMessage(index: 2, controlKind: "range_rewrite")
        let marker = controlMessage(index: 3, controlKind: "range_rewrite")

        XCTAssertEqual(GaryxStreamSeqPlanner.decide(incomingSeq: 3, connectionLastSeq: 3), .apply)

        let window = GaryxTranscriptCacheLogic.merged(
            into: GaryxCachedTranscript(
                threadId: "thread::rewrite",
                savedAt: Date(timeIntervalSince1970: 0),
                messages: [old],
                hasMoreBefore: false,
                nextBeforeIndex: nil
            ),
            threadId: "thread::rewrite",
            fetched: [tombstone],
            pageInfo: nil,
            direction: .forward,
            savedAt: Date(timeIntervalSince1970: 1)
        )
        XCTAssertEqual(window.messages.map(\.kind), ["control"])
        XCTAssertTrue(GaryxMobileTranscriptMapper.mobileMessages(from: window.messages).isEmpty)
        XCTAssertEqual(
            GaryxTranscriptControlRewritePlanner.action(for: marker),
            .refetchAuthoritativeTranscript
        )
    }

    func testTranscriptResetControlRequiresAuthoritativeRefetch() {
        XCTAssertEqual(
            GaryxTranscriptControlRewritePlanner.action(
                for: controlMessage(index: 9, controlKind: "transcript_reset")
            ),
            .refetchAuthoritativeTranscript
        )
    }

    func testOrdinaryControlDoesNotForceRefetch() {
        XCTAssertEqual(
            GaryxTranscriptControlRewritePlanner.action(
                for: controlMessage(index: 9, controlKind: "done")
            ),
            .none
        )
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

    // MARK: - Stream render window

    func testCapturedOneTurnInitialWindowRequiresLargerMobileDefault() throws {
        let frames = try Self.scrubbedCapturedInitialWindowFrames()
        XCTAssertEqual(frames.map { $0.renderState.rows.count }, [1, 1])
        XCTAssertEqual(frames.first?.renderState.window, GaryxRenderWindow(floorSeq: 2, hasMoreAbove: true))

        let cold = GaryxThreadWindowPlanner.streamRequest(
            afterSeq: 42,
            renderFloor: nil,
            hasWindowedRenderSnapshot: false
        )
        XCTAssertGreaterThanOrEqual(
            cold.initialUserTurns ?? 0,
            3,
            "A one-turn cold render window is too small and makes the top boundary appear immediately."
        )
    }

    func testThreadWindowPlannerColdReconnectScrollUpSequence() {
        let cold = GaryxThreadWindowPlanner.streamRequest(
            afterSeq: 42,
            renderFloor: nil,
            hasWindowedRenderSnapshot: false
        )
        XCTAssertEqual(cold.afterSeq, 0)
        XCTAssertEqual(cold.replayScope, .initial)
        XCTAssertEqual(cold.initialUserTurns, 3)
        XCTAssertNil(cold.renderFloor)

        let reconnect = GaryxThreadWindowPlanner.streamRequest(
            afterSeq: 9,
            renderFloor: 7,
            hasWindowedRenderSnapshot: true
        )
        XCTAssertEqual(reconnect.afterSeq, 9)
        XCTAssertEqual(reconnect.replayScope, .resume)
        XCTAssertNil(reconnect.initialUserTurns)
        XCTAssertEqual(reconnect.renderFloor, 7)

        let loweredFloor = GaryxThreadWindowPlanner.floorSeqForOlderPage(firstIndex: 3)
        XCTAssertEqual(loweredFloor, 4)

        let expandedReconnect = GaryxThreadWindowPlanner.streamRequest(
            afterSeq: 11,
            renderFloor: loweredFloor,
            hasWindowedRenderSnapshot: true
        )
        XCTAssertEqual(expandedReconnect.afterSeq, 11)
        XCTAssertEqual(expandedReconnect.replayScope, .resume)
        XCTAssertEqual(expandedReconnect.renderFloor, 4)
    }

    private static func scrubbedCapturedInitialWindowFrames() throws -> [GaryxThreadRenderFrame] {
        let json = """
        [
          {
            "type": "thread_render_frame",
            "thread_id": "thread::fixture",
            "events": [],
            "render_state": {
              "based_on_seq": 166,
              "rows": [
                {
                  "kind": "user_turn",
                  "id": "turn:captured-tail",
                  "user": { "id": "seq:2", "seq": 2, "role": "user" },
                  "activity": []
                }
              ],
              "tailActivity": "none",
              "activeToolGroupId": null,
              "progress_locus": "none",
              "visibleMessageIds": ["seq:2"],
              "filtered_placeholders": [],
              "window": { "floor_seq": 2, "has_more_above": true }
            }
          },
          {
            "type": "thread_render_frame",
            "thread_id": "thread::fixture",
            "events": [],
            "render_state": {
              "based_on_seq": 167,
              "rows": [
                {
                  "kind": "user_turn",
                  "id": "turn:captured-tail",
                  "user": { "id": "seq:2", "seq": 2, "role": "user" },
                  "activity": []
                }
              ],
              "tailActivity": "none",
              "activeToolGroupId": null,
              "progress_locus": "none",
              "visibleMessageIds": ["seq:2"],
              "filtered_placeholders": [],
              "window": { "floor_seq": 2, "has_more_above": true }
            }
          }
        ]
        """
        return try JSONDecoder().decode([GaryxThreadRenderFrame].self, from: Data(json.utf8))
    }

    private func controlMessage(index: Int, controlKind: String) -> GaryxTranscriptMessage {
        GaryxTranscriptMessage(
            index: index,
            role: .system,
            kind: "control",
            text: "",
            content: .object([
                "control": .object([
                    "kind": .string(controlKind)
                ])
            ]),
            message: .object([
                "role": .string("system"),
                "kind": .string("control"),
                "internal": .bool(true),
                "internal_kind": .string("control"),
                "control": .object([
                    "kind": .string(controlKind)
                ])
            ]),
            toolRelated: false,
            likelyUserVisible: false
        )
    }
}
