import XCTest
@testable import GaryxMobileCore

final class HomeProjectionActorTests: XCTestCase {
    func testCheckpointParityIgnoresLiveSummaryOnlyRunningMismatch() async throws {
        let fixture = GaryxHomeListFixture.makeInputs(threadCount: 20, pinnedCount: 2, runningCount: 0)
        let legacyInput = GaryxHomeThreadListInput(
            fixture,
            isLoadingThreads: false,
            isHomeVisible: true
        )
        let liveStore = GaryxHomeThreadListStore()
        XCTAssertTrue(liveStore.apply(legacyInput))
        XCTAssertFalse(
            try XCTUnwrap(liveStore.snapshot.sections.allRows.first { $0.id == "thread-10" })
                .presentation
                .isRunning
        )

        let actor = HomeProjectionActor()
        let result = await actor.applyBoundary(
            capture: HomeProjectionCapture(
                legacyInput: legacyInput,
                runTrackerBusyThreadIds: ["thread-10"]
            ),
            transactionId: 1,
            liveLegacySnapshot: liveStore.snapshot
        )

        let actorRow = try XCTUnwrap(result.snapshot.sections.allRows.first { $0.id == "thread-10" })
        XCTAssertTrue(actorRow.presentation.isRunning)
        XCTAssertEqual(result.parityMismatchCount, 0)
        XCTAssertNil(result.latestParityMismatch)
        XCTAssertEqual(result.snapshotEmitCount, 1)
        XCTAssertFalse(
            try XCTUnwrap(result.liveLegacyDiagnostics).matchesActorSnapshot,
            "The live store is summary-only today; M2 parity must treat this as diagnostics, not a gate."
        )
    }

    @MainActor
    func testExplicitTransactionCoalescesBurstToOneSnapshotEmit() async throws {
        let fixture = GaryxHomeListFixture.makeInputs(threadCount: 20, pinnedCount: 2, runningCount: 0)
        let gateway = HomeProjectionGateway(isEnabled: true)
        let transactionId = gateway.beginTransaction(label: "running-open-pop-drain")

        var openInput = fixture
        openInput.selectedThreadId = "thread-10"
        gateway.capture(HomeProjectionCapture(
            legacyInput: GaryxHomeThreadListInput(openInput, isLoadingThreads: false, isHomeVisible: false),
            runTrackerBusyThreadIds: ["thread-10"]
        ))

        var homeInput = openInput
        gateway.capture(HomeProjectionCapture(
            legacyInput: GaryxHomeThreadListInput(homeInput, isLoadingThreads: false, isHomeVisible: true),
            runTrackerBusyThreadIds: ["thread-10"]
        ))

        gateway.capture(HomeProjectionCapture(
            legacyInput: GaryxHomeThreadListInput(homeInput, isLoadingThreads: false, isHomeVisible: true),
            runTrackerBusyThreadIds: ["thread-10"]
        ))

        homeInput.selectedThreadId = nil
        gateway.capture(HomeProjectionCapture(
            legacyInput: GaryxHomeThreadListInput(homeInput, isLoadingThreads: false, isHomeVisible: true),
            runTrackerBusyThreadIds: [],
            committedRunStateBusyByThreadId: ["thread-10": false]
        ))

        gateway.endTransaction(transactionId)
        await gateway.waitForIdleForTesting()

        let result = try XCTUnwrap(gateway.latestResult)
        XCTAssertEqual(result.snapshotEmitCount, 1)
        XCTAssertEqual(gateway.snapshotEmitCount, 1)
        XCTAssertEqual(result.parityMismatchCount, 0)
        XCTAssertEqual(result.snapshot.sections.allRows.filter { $0.presentation.isRunning }.count, 0)
        XCTAssertTrue(result.snapshot.isHomeVisible)
    }

    @MainActor
    func testGatewayUsesLatestBoundaryWhileActorIsInFlight() async throws {
        let fixture = GaryxHomeListFixture.makeInputs(threadCount: 40, pinnedCount: 4, runningCount: 0)
        let gateway = HomeProjectionGateway(isEnabled: true)

        var first = fixture
        first.selectedThreadId = "thread-1"
        gateway.capture(HomeProjectionCapture(
            legacyInput: GaryxHomeThreadListInput(first, isLoadingThreads: false, isHomeVisible: true)
        ))

        var latest = fixture
        latest.selectedThreadId = "thread-30"
        latest.busyThreadIds = ["thread-30"]
        gateway.capture(HomeProjectionCapture(
            legacyInput: GaryxHomeThreadListInput(latest, isLoadingThreads: true, isHomeVisible: false),
            committedRunStateBusyByThreadId: ["thread-30": true]
        ))

        await gateway.waitForIdleForTesting()

        let result = try XCTUnwrap(gateway.latestResult)
        XCTAssertLessThanOrEqual(result.snapshotEmitCount, 2)
        XCTAssertEqual(result.parityMismatchCount, 0)
        let selectedRows = result.snapshot.sections.allRows.filter { $0.presentation.isSelected }
        XCTAssertEqual(selectedRows.map(\.id), ["thread-30"])
        XCTAssertTrue(
            try XCTUnwrap(result.snapshot.sections.allRows.first { $0.id == "thread-30" })
                .presentation
                .isRunning
        )
        XCTAssertTrue(result.snapshot.isLoadingThreads)
        XCTAssertFalse(result.snapshot.isHomeVisible)
    }

    @MainActor
    func testDisabledGatewayDropsCapturesAndTransactions() async {
        let fixture = GaryxHomeListFixture.makeInputs(threadCount: 10, pinnedCount: 1, runningCount: 0)
        let gateway = HomeProjectionGateway(isEnabled: false)

        XCTAssertNil(gateway.beginTransaction(label: "disabled"))
        gateway.capture(HomeProjectionCapture(
            legacyInput: GaryxHomeThreadListInput(fixture, isLoadingThreads: false, isHomeVisible: true)
        ))
        gateway.endTransaction(nil)
        await gateway.waitForIdleForTesting()

        XCTAssertNil(gateway.latestResult)
        XCTAssertEqual(gateway.snapshotEmitCount, 0)
        XCTAssertEqual(gateway.parityMismatchCount, 0)
    }
}

private extension GaryxHomeThreadSectionsInput {
    init(_ input: HomeThreadSectionsReference.Inputs) {
        self.init(
            threads: input.threads,
            agents: input.agents,
            teams: input.teams,
            automations: input.automations,
            pinnedThreadIds: input.pinnedThreadIds,
            recentThreadIds: input.recentThreadIds,
            selectedThreadId: input.selectedThreadId
        )
    }
}

private extension GaryxHomeThreadListInput {
    init(
        _ input: HomeThreadSectionsReference.Inputs,
        isLoadingThreads: Bool,
        isHomeVisible: Bool
    ) {
        self.init(
            sectionsInput: GaryxHomeThreadSectionsInput(input),
            runningThreadIds: input.busyThreadIds,
            isLoadingThreads: isLoadingThreads,
            isHomeVisible: isHomeVisible
        )
    }
}
