import XCTest
@testable import GaryxMobileCore

final class GaryxPendingThreadArchiveStateTests: XCTestCase {
    func testInFlightArchiveKeepsRowsVisibleAndCoalescesDuplicateStart() {
        var state = GaryxPendingThreadArchiveState()
        XCTAssertTrue(state.startArchive(threadId: " thread::archive-me "))
        XCTAssertFalse(state.startArchive(threadId: "thread::archive-me"))

        XCTAssertEqual(
            state.visibleThreads([
                thread("thread::keep-a"),
                thread("thread::archive-me"),
                thread("thread::keep-b"),
            ]).map(\.id),
            ["thread::keep-a", "thread::archive-me", "thread::keep-b"]
        )
        XCTAssertEqual(
            state.visibleThreadIds([
                "thread::archive-me",
                "thread::keep-a",
                " thread::archive-me ",
                "thread::keep-b",
            ]),
            [
                "thread::archive-me",
                "thread::keep-a",
                " thread::archive-me ",
                "thread::keep-b",
            ]
        )
        XCTAssertTrue(state.isRequestInFlight(threadId: "thread::archive-me"))
        XCTAssertFalse(state.isCommitted(threadId: "thread::archive-me"))
    }

    func testCommittedArchiveFiltersStaleRefreshRowsAndThreadIds() {
        let archived = thread("thread::archive-me")
        var state = GaryxPendingThreadArchiveState()
        state.startArchive(threadId: archived.id)
        state.commitArchive(threadId: archived.id)

        XCTAssertTrue(state.contains(threadId: archived.id))
        XCTAssertFalse(state.isRequestInFlight(threadId: archived.id))
        XCTAssertTrue(state.isCommitted(threadId: archived.id))
        XCTAssertEqual(
            state.visibleThreads([thread("thread::keep"), archived]).map(\.id),
            ["thread::keep"]
        )
        XCTAssertEqual(state.visibleThreadIds([archived.id, "thread::keep"]), ["thread::keep"])
        XCTAssertFalse(state.startArchive(threadId: archived.id))
    }

    func testCancelledArchiveLeavesRowsVisibleAndAllowsRetry() {
        let archived = thread("thread::archive-me")
        var state = GaryxPendingThreadArchiveState()
        state.startArchive(threadId: archived.id)
        state.cancelArchive(threadId: archived.id)

        XCTAssertFalse(state.contains(threadId: archived.id))
        XCTAssertEqual(state.visibleThreads([archived]), [archived])
        XCTAssertEqual(state.visibleThreadIds([archived.id]), [archived.id])
        XCTAssertTrue(state.startArchive(threadId: archived.id))
    }

    private func thread(_ id: String) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: id,
            title: id,
            createdAt: nil,
            updatedAt: nil,
            lastMessagePreview: "",
            workspacePath: nil,
            messageCount: nil,
            agentId: nil,
            providerType: nil,
            recentRunId: nil,
            activeRunId: nil,
            runState: nil,
            worktreePath: nil
        )
    }
}
