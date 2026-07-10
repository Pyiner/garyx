import XCTest
@testable import GaryxMobileCore

final class GaryxCompletedThreadHydrationPolicyTests: XCTestCase {
    func testHydratesWhenPreviouslyRunningThreadCompleted() {
        XCTAssertTrue(
            GaryxCompletedThreadHydrationPolicy.shouldHydrate(
                previousThread: thread(id: "t-1", runState: "running"),
                previousRemoteBusyThreadIds: [],
                refreshedThread: thread(id: "t-1", runState: "idle"),
                selectedThreadId: "t-other"
            )
        )
    }

    func testHydratesWhenPreviouslyRemoteBusyThreadCompleted() {
        XCTAssertTrue(
            GaryxCompletedThreadHydrationPolicy.shouldHydrate(
                previousThread: nil,
                previousRemoteBusyThreadIds: ["t-1"],
                refreshedThread: thread(id: "t-1", runState: nil),
                selectedThreadId: nil
            )
        )
    }

    func testSkipsSelectedThread() {
        XCTAssertFalse(
            GaryxCompletedThreadHydrationPolicy.shouldHydrate(
                previousThread: thread(id: "t-1", runState: "running"),
                previousRemoteBusyThreadIds: ["t-1"],
                refreshedThread: thread(id: "t-1", runState: nil),
                selectedThreadId: "t-1"
            )
        )
    }

    func testSkipsStillRunningThread() {
        XCTAssertFalse(
            GaryxCompletedThreadHydrationPolicy.shouldHydrate(
                previousThread: thread(id: "t-1", runState: "running"),
                previousRemoteBusyThreadIds: ["t-1"],
                refreshedThread: thread(id: "t-1", runState: "running"),
                selectedThreadId: nil
            )
        )
    }

    func testSkipsThreadWithoutPriorRunningObservation() {
        XCTAssertFalse(
            GaryxCompletedThreadHydrationPolicy.shouldHydrate(
                previousThread: thread(id: "t-1", runState: "idle"),
                previousRemoteBusyThreadIds: [],
                refreshedThread: thread(id: "t-1", runState: nil),
                selectedThreadId: nil
            )
        )
    }

    func testSkipsEmptyThreadId() {
        XCTAssertFalse(
            GaryxCompletedThreadHydrationPolicy.shouldHydrate(
                previousThread: thread(id: "  ", runState: "running"),
                previousRemoteBusyThreadIds: [" "],
                refreshedThread: thread(id: "  ", runState: nil),
                selectedThreadId: nil
            )
        )
    }

    func testIsRunningMatchesTrimmedCaseInsensitiveRunning() {
        XCTAssertTrue(GaryxThreadSummaryRunStateResolver.isRunning(thread(id: "t", runState: "running")))
        XCTAssertTrue(GaryxThreadSummaryRunStateResolver.isRunning(thread(id: "t", runState: " Running ")))
    }

    func testIsRunningFalseForIdleOrMissingState() {
        XCTAssertFalse(GaryxThreadSummaryRunStateResolver.isRunning(thread(id: "t", runState: "idle")))
        XCTAssertFalse(GaryxThreadSummaryRunStateResolver.isRunning(thread(id: "t", runState: nil)))
        XCTAssertFalse(GaryxThreadSummaryRunStateResolver.isRunning(thread(id: "t", runState: "")))
    }

    private func thread(id: String, runState: String?) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: id,
            title: "Thread",
            createdAt: nil,
            updatedAt: nil,
            lastMessagePreview: "",
            workspacePath: nil,
            messageCount: nil,
            agentId: nil,
            providerType: nil,
            recentRunId: nil,
            activeRunId: nil,
            runState: runState,
            worktreePath: nil
        )
    }
}
