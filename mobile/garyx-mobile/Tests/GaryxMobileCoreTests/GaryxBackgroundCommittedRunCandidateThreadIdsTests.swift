import XCTest
@testable import GaryxMobileCore

final class GaryxBackgroundCommittedRunCandidateThreadIdsTests: XCTestCase {
    func testUnionsTrackedCommittedBusyAndSummaryRunningThreads() {
        let candidates = GaryxBackgroundCommittedRunReconcilePlanner.candidateThreadIds(
            locallyTrackedThreadIds: ["t-local"],
            runStateByThread: [
                "t-busy": runState(busy: true),
                "t-idle": runState(busy: false),
            ],
            threads: [
                thread(id: "t-idle", runState: "running"),
                thread(id: "t-summary-running", runState: "running"),
                thread(id: "t-quiet", runState: nil),
            ],
            selectedThreadId: nil
        )
        XCTAssertEqual(candidates, ["t-busy", "t-local", "t-summary-running"])
    }

    func testCommittedIdleStateVetoesSummaryRunningThread() {
        let candidates = GaryxBackgroundCommittedRunReconcilePlanner.candidateThreadIds(
            locallyTrackedThreadIds: [],
            runStateByThread: ["t-1": runState(busy: false)],
            threads: [thread(id: "t-1", runState: "running")],
            selectedThreadId: nil
        )
        XCTAssertEqual(candidates, [])
    }

    func testExcludesSelectedThread() {
        let candidates = GaryxBackgroundCommittedRunReconcilePlanner.candidateThreadIds(
            locallyTrackedThreadIds: ["t-selected", "t-other"],
            runStateByThread: [:],
            threads: [],
            selectedThreadId: " t-selected "
        )
        XCTAssertEqual(candidates, ["t-other"])
    }

    func testDropsEmptyIdsAndSortsResult() {
        let candidates = GaryxBackgroundCommittedRunReconcilePlanner.candidateThreadIds(
            locallyTrackedThreadIds: ["  ", "t-b"],
            runStateByThread: ["t-a": runState(busy: true)],
            threads: [],
            selectedThreadId: nil
        )
        XCTAssertEqual(candidates, ["t-a", "t-b"])
    }

    private func runState(busy: Bool) -> GaryxTranscriptRunState {
        GaryxTranscriptRunState(busy: busy, activeRunId: busy ? "run-1" : nil)
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
            teamId: nil,
            teamName: nil,
            providerType: nil,
            recentRunId: nil,
            activeRunId: nil,
            runState: runState,
            worktreePath: nil
        )
    }
}
