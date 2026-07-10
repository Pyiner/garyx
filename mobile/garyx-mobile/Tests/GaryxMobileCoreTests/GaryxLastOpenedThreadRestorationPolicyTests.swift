import XCTest
@testable import GaryxMobileCore

final class GaryxLastOpenedThreadRestorationPolicyTests: XCTestCase {
    func testWorkflowRunFromSimulatorCaptureIsNotARestorableColdLaunchThread() throws {
        let capture = try Self.simulatorCapture()
        let chatThread = capture.gatewayRecords.chatThread.threadSummary()
        let workflowThread = capture.gatewayRecords.workflowThread.threadSummary()
        let workflowDestination = GaryxWorkflowRunDestination.destination(for: workflowThread)

        XCTAssertEqual(capture.capture.simulator.name, "Garyx Mobile UI QA")
        XCTAssertEqual(capture.gatewayRecords.workflowRun.eventCount, 3)
        XCTAssertEqual(capture.gatewayRecords.workflowEvents.eventTypes, [
            "workflow.created",
            "smoke.started",
            "workflow.completed",
        ])
        XCTAssertEqual(capture.stateSequence.chatBackground.lastOpenedThreadId, chatThread.id)
        XCTAssertTrue(capture.stateSequence.chatBackground.lastSessionOnThread)
        XCTAssertEqual(capture.stateSequence.workflowOpenAfterChatBackground.lastOpenedThreadId, workflowThread.id)
        XCTAssertTrue(capture.stateSequence.workflowOpenAfterChatBackground.lastSessionOnThread)
        XCTAssertEqual(capture.stateSequence.coldLaunchAfterPollutedWorkflowOpen.restoredSurface, "workflowRun")
        XCTAssertEqual(
            capture.stateSequence.coldLaunchAfterPollutedWorkflowOpen.restoredUITitle,
            capture.gatewayRecords.workflowRun.title
        )
        XCTAssertFalse(capture.stateSequence.workflowBackgroundControl.lastSessionOnThread)
        XCTAssertEqual(workflowDestination, .workflowRun(runId: workflowThread.id))
        XCTAssertEqual(
            GaryxLastOpenedThreadRestorationPolicy.persistedThreadId(
                afterOpening: .chat(threadId: chatThread.id),
                previousThreadId: nil
            ),
            chatThread.id
        )
        XCTAssertEqual(
            GaryxLastOpenedThreadRestorationPolicy.persistedThreadId(
                afterOpening: workflowDestination,
                previousThreadId: capture.stateSequence.chatBackground.lastOpenedThreadId
            ),
            chatThread.id,
            "Workflow-run surfaces are not thread conversations and must not overwrite the last-opened thread slot."
        )
        XCTAssertFalse(
            GaryxLastOpenedThreadRestorationPolicy.isSessionRestorableAfterOpening(workflowDestination),
            "Opening a workflow-run surface must immediately make the persisted launch location non-restorable."
        )
        XCTAssertNil(
            GaryxLastOpenedThreadRestorationPolicy.restoreThreadId(
                persistedLastOpenedThreadId: capture.stateSequence.coldLaunchAfterPollutedWorkflowOpen.lastOpenedThreadId,
                persistedLastSessionWasOnThread: capture.stateSequence
                    .coldLaunchAfterPollutedWorkflowOpen
                    .lastSessionOnThread,
                selectedThreadId: nil,
                hasPendingMobileRoute: false,
                hasPendingThreadIntent: false,
                navigationState: GaryxMobileNavigationState(),
                sidebarVisible: false,
                resolvedDestination: workflowDestination
            ),
            "Even a previously polluted last-opened slot must not restore a workflow-run destination on cold launch."
        )
    }

    func testInitialEmptyLoadingSnapshotDerivesRecentSkeletonRowsInCore() {
        let store = GaryxHomeThreadListStore()
        let input = GaryxHomeThreadListInput(
            sectionsInput: GaryxHomeThreadSectionsInput(
                threads: [],
                agents: [],
                automations: [],
                pinnedThreadIds: [],
                recentThreadIds: [],
                selectedThreadId: nil
            ),
            runningThreadIds: [],
            isLoadingThreads: true,
            isHomeVisible: true
        )

        XCTAssertTrue(store.apply(input))
        XCTAssertEqual(store.snapshot.recentPlaceholder, .loadingSkeleton(rowCount: 6))
    }

    func testCachedRecentRowsSuppressSkeletonDuringRefresh() {
        let fixture = GaryxHomeListFixture.makeInputs(threadCount: 3, pinnedCount: 0, runningCount: 0)
        let store = GaryxHomeThreadListStore()
        let input = GaryxHomeThreadListInput(
            sectionsInput: GaryxHomeThreadSectionsInput(
                threads: fixture.threads,
                agents: fixture.agents,
                automations: fixture.automations,
                pinnedThreadIds: fixture.pinnedThreadIds,
                recentThreadIds: fixture.recentThreadIds,
                selectedThreadId: fixture.selectedThreadId
            ),
            runningThreadIds: [],
            isLoadingThreads: true,
            isHomeVisible: true
        )

        XCTAssertTrue(store.apply(input))
        XCTAssertEqual(store.snapshot.sections.recent.count, 3)
        XCTAssertEqual(store.snapshot.recentPlaceholder, .none)
    }

    private static func simulatorCapture() throws -> MobileColdStartSimulatorCapture {
        try JSONDecoder().decode(
            MobileColdStartSimulatorCapture.self,
            from: simulatorCaptureData()
        )
    }

    private static func simulatorCaptureData() throws -> Data {
        let fileURL = URL(fileURLWithPath: #filePath)
        let testsDirectory = fileURL
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let fixtureURL = testsDirectory
            .appendingPathComponent("Fixtures")
            .appendingPathComponent("mobile-cold-start-workflow-restore-capture.json")
        return try Data(contentsOf: fixtureURL)
    }
}

private struct MobileColdStartSimulatorCapture: Decodable {
    var capture: CaptureMetadata
    var gatewayRecords: GatewayRecords
    var stateSequence: StateSequence

    struct CaptureMetadata: Decodable {
        var simulator: Simulator

        struct Simulator: Decodable {
            var name: String
        }
    }

    struct GatewayRecords: Decodable {
        var chatThread: CapturedThreadSummary
        var workflowThread: CapturedThreadSummary
        var workflowRun: WorkflowRun
        var workflowEvents: WorkflowEvents
    }

    struct WorkflowRun: Decodable {
        var title: String
        var eventCount: Int
    }

    struct WorkflowEvents: Decodable {
        var eventTypes: [String]
    }

    struct StateSequence: Decodable {
        var chatBackground: PersistedLocation
        var workflowOpenAfterChatBackground: PersistedLocation
        var coldLaunchAfterPollutedWorkflowOpen: RestoredPersistedLocation
        var workflowBackgroundControl: PersistedLocation
    }

    struct PersistedLocation: Decodable {
        var lastOpenedThreadId: String
        var lastSessionOnThread: Bool
    }

    struct RestoredPersistedLocation: Decodable {
        var lastOpenedThreadId: String
        var lastSessionOnThread: Bool
        var restoredSurface: String
        var restoredUITitle: String
    }
}

private struct CapturedThreadSummary: Decodable {
    var id: String
    var title: String
    var createdAt: String?
    var updatedAt: String?
    var lastMessagePreview: String
    var workspacePath: String?
    var messageCount: Int?
    var agentId: String?
    var providerType: String?
    var recentRunId: String?
    var activeRunId: String?
    var runState: String?
    var worktreePath: String?
    var threadType: String?
    var workflowRunId: String?

    func threadSummary() -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: id,
            title: title,
            createdAt: createdAt,
            updatedAt: updatedAt,
            lastMessagePreview: lastMessagePreview,
            workspacePath: workspacePath,
            messageCount: messageCount,
            agentId: agentId,
            providerType: providerType,
            recentRunId: recentRunId,
            activeRunId: activeRunId,
            runState: runState,
            worktreePath: worktreePath,
            threadType: threadType,
            workflowRunId: workflowRunId
        )
    }
}
