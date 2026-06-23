import XCTest
@testable import GaryxMobileCore

final class GaryxSelectedThreadHistoryPresentationTests: XCTestCase {
    func testLoadedCommittedHistoryWithoutRenderSnapshotStillAwaitsInitialHistory() {
        XCTAssertTrue(isAwaiting(
            historyLoaded: true,
            cachedTranscript: cachedTranscript(messages: [
                message(index: 0, role: .user, text: "Question"),
                message(index: 1, role: .assistant, text: "Answer"),
            ])
        ))
    }

    func testTrueEmptyLoadedThreadDoesNotAwaitInitialHistory() {
        XCTAssertFalse(isAwaiting(
            historyLoaded: true,
            cachedTranscript: cachedTranscript(messages: [])
        ))
    }

    func testOldGatewayFallbackWithCommittedHistoryDoesNotShowEmptyState() {
        XCTAssertTrue(isAwaiting(
            historyLoaded: true,
            cachedTranscript: cachedTranscript(messages: [
                message(index: 0, role: .user, text: "Persisted")
            ])
        ))
    }

    func testLoadedRemoteFinalMessagesWithoutCachedTranscriptAwaitInitialHistory() {
        XCTAssertTrue(isAwaiting(
            historyLoaded: true,
            cachedTranscript: nil,
            hasRemoteFinalMessages: true
        ))
    }

    func testInternalOnlyCommittedHistoryDoesNotAwaitInitialHistory() {
        XCTAssertFalse(isAwaiting(
            historyLoaded: true,
            cachedTranscript: cachedTranscript(messages: [
                message(index: 0, role: .system, internalMessage: true),
            ])
        ))
    }

    func testToolOnlyCommittedHistoryAwaitsInitialHistory() {
        XCTAssertTrue(isAwaiting(
            historyLoaded: true,
            cachedTranscript: cachedTranscript(messages: [
                message(index: 0, role: .toolUse, text: #"{"tool":"Bash"}"#),
                message(index: 1, role: .toolResult, text: #"{"stdout":"ok"}"#),
            ])
        ))
    }

    func testLiveRenderSnapshotStopsAwaitingInitialHistory() {
        XCTAssertFalse(isAwaiting(
            historyLoaded: false,
            liveRenderSnapshot: renderSnapshot(),
            cachedTranscript: cachedTranscript(messages: [
                message(index: 0, role: .user, text: "Question")
            ])
        ))
    }

    func testLiveRenderSnapshotWithUnresolvedRefsStillAwaitsInitialHistory() {
        XCTAssertTrue(isAwaiting(
            historyLoaded: true,
            liveRenderSnapshot: renderSnapshot(),
            cachedTranscript: cachedTranscript(messages: [])
        ))
    }

    func testLiveRenderSnapshotResolvedByMobileMessagesStopsAwaitingInitialHistory() {
        XCTAssertFalse(isAwaiting(
            historyLoaded: true,
            liveRenderSnapshot: renderSnapshot(),
            cachedTranscript: cachedTranscript(messages: []),
            resolvedHistoryIndexes: [0]
        ))
    }

    func testCachedRenderSnapshotStopsAwaitingInitialHistory() {
        XCTAssertFalse(isAwaiting(
            historyLoaded: true,
            cachedTranscript: cachedTranscript(
                messages: [message(index: 0, role: .user, text: "Question")],
                renderSnapshot: renderSnapshot()
            )
        ))
    }

    func testUnloadedThreadAwaitsInitialHistory() {
        XCTAssertTrue(isAwaiting(
            historyLoaded: false,
            cachedTranscript: nil
        ))
    }

    func testBlankThreadIdDoesNotAwaitInitialHistory() {
        XCTAssertFalse(isAwaiting(
            threadId: "   ",
            historyLoaded: false,
            cachedTranscript: cachedTranscript(messages: [
                message(index: 0, role: .user, text: "Question")
            ])
        ))
    }

    private func isAwaiting(
        threadId: String? = "thread::history",
        historyLoaded: Bool,
        liveRenderSnapshot: GaryxRenderSnapshot? = nil,
        cachedTranscript: GaryxCachedTranscript?,
        resolvedMessageIds: Set<String> = [],
        resolvedHistoryIndexes: Set<Int> = [],
        hasRemoteFinalMessages: Bool = false
    ) -> Bool {
        GaryxSelectedThreadHistoryPresentation.isAwaitingInitialHistory(
            threadId: threadId,
            historyLoaded: historyLoaded,
            liveRenderSnapshot: liveRenderSnapshot,
            cachedTranscript: cachedTranscript,
            resolvedMessageIds: resolvedMessageIds,
            resolvedHistoryIndexes: resolvedHistoryIndexes,
            hasRemoteFinalMessages: hasRemoteFinalMessages
        )
    }

    private func cachedTranscript(
        messages: [GaryxTranscriptMessage],
        renderSnapshot: GaryxRenderSnapshot? = nil
    ) -> GaryxCachedTranscript {
        GaryxCachedTranscript(
            threadId: "thread::history",
            savedAt: Date(timeIntervalSince1970: 0),
            messages: messages,
            renderSnapshot: renderSnapshot,
            hasMoreBefore: false,
            nextBeforeIndex: nil
        )
    }

    private func message(
        index: Int,
        role: GaryxTranscriptRole,
        text: String = "",
        internalMessage: Bool = false
    ) -> GaryxTranscriptMessage {
        GaryxTranscriptMessage(
            index: index,
            role: role,
            internalMessage: internalMessage,
            text: text
        )
    }

    private func renderSnapshot() -> GaryxRenderSnapshot {
        GaryxRenderSnapshot(
            basedOnSeq: 1,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "turn:1",
                    user: GaryxRenderMessageRef(id: "seq:1", seq: 1, role: "user"),
                    activity: []
                )),
            ]
        )
    }
}
