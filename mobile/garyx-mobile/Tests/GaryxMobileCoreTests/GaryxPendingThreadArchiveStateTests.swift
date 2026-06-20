import XCTest
@testable import GaryxMobileCore

final class GaryxPendingThreadArchiveStateTests: XCTestCase {
    func testPendingArchiveFiltersRefreshRowsAndThreadIds() {
        var state = GaryxPendingThreadArchiveState()
        state.startArchive(threadId: " thread::archive-me ")

        XCTAssertEqual(
            state.visibleThreads([
                thread("thread::keep-a"),
                thread("thread::archive-me"),
                thread("thread::keep-b"),
            ]).map(\.id),
            ["thread::keep-a", "thread::keep-b"]
        )
        XCTAssertEqual(
            state.visibleThreadIds([
                "thread::archive-me",
                "thread::keep-a",
                " thread::archive-me ",
                "thread::keep-b",
            ]),
            ["thread::keep-a", "thread::keep-b"]
        )
    }

    func testResolvingArchiveAllowsRefreshRowsAgain() {
        let archived = thread("thread::archive-me")
        var state = GaryxPendingThreadArchiveState()
        state.startArchive(threadId: archived.id)

        XCTAssertTrue(state.contains(threadId: archived.id))
        state.resolveArchive(threadId: archived.id)

        XCTAssertFalse(state.contains(threadId: archived.id))
        XCTAssertEqual(state.visibleThreads([archived]), [archived])
        XCTAssertEqual(state.visibleThreadIds([archived.id]), [archived.id])
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
            teamId: nil,
            teamName: nil,
            providerType: nil,
            recentRunId: nil,
            activeRunId: nil,
            runState: nil,
            worktreePath: nil
        )
    }
}
