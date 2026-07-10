import XCTest
@testable import GaryxMobileCore

final class GaryxThreadSummaryCommittedRunStateProjectorTests: XCTestCase {
    func testNilCommittedStateStripsActiveRunAndKeepsSummaryFields() throws {
        let runtime = try runtimeWithActiveRun()
        XCTAssertNotNil(runtime.activeRun)
        let projected = GaryxThreadSummaryCommittedRunStateProjector.summary(
            thread(activeRunId: "api-run", runState: "running", threadRuntime: runtime),
            committedState: nil
        )
        XCTAssertNotNil(projected.threadRuntime)
        XCTAssertNil(projected.threadRuntime?.activeRun)
        XCTAssertEqual(projected.activeRunId, "api-run")
        XCTAssertEqual(projected.runState, "running")
        XCTAssertEqual(projected.title, "Thread")
    }

    func testBusyCommittedStateProjectsActiveRunIdAndTitle() throws {
        let state = GaryxTranscriptRunState(
            busy: true,
            activeRunId: "run-9",
            title: "  Renamed Thread  "
        )
        let projected = GaryxThreadSummaryCommittedRunStateProjector.summary(
            thread(activeRunId: "api-run", runState: "idle", threadRuntime: try runtimeWithActiveRun()),
            applying: state
        )
        XCTAssertEqual(projected.activeRunId, "run-9")
        XCTAssertEqual(projected.title, "Renamed Thread")
        XCTAssertEqual(projected.runState, "idle")
        XCTAssertNil(projected.threadRuntime?.activeRun)
    }

    func testIdleCommittedStateClearsActiveRunIdAndKeepsTitle() {
        let projected = GaryxThreadSummaryCommittedRunStateProjector.summary(
            thread(activeRunId: "api-run", runState: "running", threadRuntime: nil),
            applying: GaryxTranscriptRunState(busy: false, activeRunId: nil, title: "   ")
        )
        XCTAssertNil(projected.activeRunId)
        XCTAssertEqual(projected.title, "Thread")
        XCTAssertNil(projected.threadRuntime)
    }

    func testCommittedStateLookupMatchesApplyingVariant() {
        let state = GaryxTranscriptRunState(busy: true, activeRunId: "run-1")
        let subject = thread(activeRunId: nil, runState: nil, threadRuntime: nil)
        XCTAssertEqual(
            GaryxThreadSummaryCommittedRunStateProjector.summary(subject, committedState: state),
            GaryxThreadSummaryCommittedRunStateProjector.summary(subject, applying: state)
        )
    }

    private func runtimeWithActiveRun() throws -> GaryxThreadRuntimeSummary {
        try JSONDecoder().decode(
            GaryxThreadRuntimeSummary.self,
            from: Data(#"{"agent_id":"agent-1","active_run":{"run_id":"run-1"}}"#.utf8)
        )
    }

    private func thread(
        activeRunId: String?,
        runState: String?,
        threadRuntime: GaryxThreadRuntimeSummary?
    ) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: "t-1",
            title: "Thread",
            createdAt: nil,
            updatedAt: nil,
            lastMessagePreview: "",
            workspacePath: nil,
            messageCount: nil,
            agentId: nil,
            providerType: nil,
            recentRunId: "recent-run",
            activeRunId: activeRunId,
            runState: runState,
            worktreePath: nil,
            threadRuntime: threadRuntime
        )
    }
}
