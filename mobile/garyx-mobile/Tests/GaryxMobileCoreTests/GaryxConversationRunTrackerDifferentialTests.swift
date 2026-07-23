import XCTest
@testable import GaryxMobileCore

final class GaryxConversationRunTrackerDifferentialTests: XCTestCase {
    private static let fixedLocalTimestamp = "2026-01-01 00:00:00"

    func testDurableBarrierRollbackRestoresBusyRuntimeAndCancelsOnlyNewIntent() {
        var tracker = GaryxConversationRunTracker()

        XCTAssertTrue(tracker.beginLocalDispatch(
            threadId: "t1",
            intentId: "i1",
            text: "start",
            clientTimestampLocal: Self.fixedLocalTimestamp
        ))
        tracker.confirmChatStartAccepted(
            requestedThreadId: "t1",
            acceptedThreadId: "t1",
            intentId: "i1",
            runId: "run-1"
        )
        let previousRuntime = tracker.machine.threadRuntimeByThread["t1"]

        XCTAssertTrue(tracker.beginLocalDispatch(
            threadId: "t1",
            intentId: "i2",
            text: "follow up",
            allowWhileBusy: true,
            clientTimestampLocal: Self.fixedLocalTimestamp
        ))
        tracker.rollbackLocalDispatch(
            threadId: "t1",
            intentId: "i2",
            previousRuntime: previousRuntime
        )

        XCTAssertEqual(tracker.machine.intentsById["i1"]?.state, .remoteAccepted)
        XCTAssertEqual(tracker.machine.intentsById["i2"]?.state, .cancelled)
        XCTAssertEqual(tracker.machine.threadRuntimeByThread["t1"], previousRuntime)
        XCTAssertTrue(tracker.isThreadBusy("t1"))
    }

    func testCommittedRunCompleteClearsLocalDispatchWithoutStreamEvent() {
        var tracker = GaryxConversationRunTracker()

        XCTAssertTrue(tracker.beginLocalDispatch(
            threadId: "t1",
            intentId: "i1",
            text: "hello",
            clientTimestampLocal: Self.fixedLocalTimestamp
        ))
        tracker.confirmChatStartAccepted(
            requestedThreadId: "t1",
            acceptedThreadId: "t1",
            intentId: "i1",
            runId: "run-1"
        )
        XCTAssertTrue(tracker.isThreadBusy("t1"))
        XCTAssertEqual(tracker.machine.intentsById["i1"]?.state, .remoteAccepted)

        tracker.completeCommittedRun(threadId: "t1")

        XCTAssertFalse(tracker.isThreadBusy("t1"))
        XCTAssertTrue(tracker.locallyTrackedThreadIds.isEmpty)
        XCTAssertEqual(tracker.machine.intentsById["i1"]?.state, .completed)
        XCTAssertEqual(tracker.machine.threadRuntimeByThread["t1"]?.state, .idle)
    }

    func testCommittedUserAckClearsQueuedPendingAckWithoutClosingRun() {
        var tracker = GaryxConversationRunTracker()

        XCTAssertTrue(tracker.beginLocalDispatch(
            threadId: "t1",
            intentId: "i1",
            text: "start",
            clientTimestampLocal: Self.fixedLocalTimestamp
        ))
        tracker.confirmChatStartAccepted(
            requestedThreadId: "t1",
            acceptedThreadId: "t1",
            intentId: "i1",
            runId: "run-1"
        )
        tracker.beginComposerSteer(
            threadId: "t1",
            intentId: "q1",
            text: "follow up",
            clientTimestampLocal: Self.fixedLocalTimestamp
        )
        XCTAssertEqual(tracker.machine.intentsById["q1"]?.source, .composerSteer)
        XCTAssertEqual(tracker.machine.intentsById["q1"]?.dispatchMode, .asyncSteer)
        XCTAssertEqual(tracker.machine.intentsById["q1"]?.state, .dispatching)
        XCTAssertEqual(tracker.machine.queueByThread["t1"] ?? [], [])
        tracker.confirmQueuedSteerAccepted(
            threadId: "t1",
            intentId: "q1",
            pendingInputId: "pending-1"
        )
        XCTAssertEqual(tracker.pendingAckIntentIdsByThread["t1"], ["q1"])

        tracker.acknowledgeProviderInput(threadId: "t1", pendingInputId: "pending-1")

        XCTAssertNil(tracker.pendingAckIntentIdsByThread["t1"])
        XCTAssertTrue(tracker.isThreadBusy("t1"))
        XCTAssertEqual(tracker.machine.intentsById["q1"]?.state, .awaitingProviderAck)
    }

    func testBackgroundCommittedTerminalClosesQueuedIntentWithoutGlobalStream() {
        var tracker = GaryxConversationRunTracker()

        tracker.beginComposerSteer(
            threadId: "background-thread",
            intentId: "q1",
            text: "queued",
            clientTimestampLocal: Self.fixedLocalTimestamp
        )
        tracker.confirmQueuedSteerAccepted(
            threadId: "background-thread",
            intentId: "q1",
            pendingInputId: "pending-1"
        )
        XCTAssertTrue(tracker.isThreadBusy("background-thread"))
        XCTAssertEqual(tracker.locallyTrackedThreadIds, Set(["background-thread"]))

        tracker.acknowledgeProviderInput(threadId: "background-thread", pendingInputId: "pending-1")
        tracker.completeCommittedRun(threadId: "background-thread")

        XCTAssertFalse(tracker.isThreadBusy("background-thread"))
        XCTAssertTrue(tracker.locallyTrackedThreadIds.isEmpty)
        XCTAssertEqual(tracker.machine.intentsById["q1"]?.state, .completed)
        XCTAssertEqual(tracker.machine.threadRuntimeByThread["background-thread"]?.state, .idle)
    }

    func testCommittedInterruptClearsRuntimeAndMarksIntentInterrupted() {
        var tracker = GaryxConversationRunTracker()

        XCTAssertTrue(tracker.beginLocalDispatch(
            threadId: "t1",
            intentId: "i1",
            text: "start",
            clientTimestampLocal: Self.fixedLocalTimestamp
        ))
        tracker.confirmChatStartAccepted(
            requestedThreadId: "t1",
            acceptedThreadId: "t1",
            intentId: "i1",
            runId: "run-1"
        )

        tracker.interruptConfirmed(threadId: "t1")

        XCTAssertFalse(tracker.isThreadBusy("t1"))
        XCTAssertTrue(tracker.locallyTrackedThreadIds.isEmpty)
        XCTAssertEqual(tracker.machine.intentsById["i1"]?.state, .interrupted)
        XCTAssertEqual(tracker.machine.threadRuntimeByThread["t1"]?.state, .idle)
    }
}
