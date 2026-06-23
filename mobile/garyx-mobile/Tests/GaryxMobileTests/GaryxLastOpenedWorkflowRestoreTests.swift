import XCTest
@testable import GaryxMobile

@MainActor
final class GaryxLastOpenedWorkflowRestoreTests: XCTestCase {
    func testWorkflowRunOpeningDoesNotPolluteLastOpenedThreadSlotAfterChatSession() throws {
        let capture = try Self.simulatorCapture()
        let model = makeModel()
        let chatThread = capture.gatewayRecords.chatThread.threadSummary()
        let workflowThread = capture.gatewayRecords.workflowThread.threadSummary()

        XCTAssertEqual(capture.stateSequence.chatBackground.lastOpenedThreadId, chatThread.id)
        XCTAssertTrue(capture.stateSequence.chatBackground.lastSessionOnThread)
        XCTAssertEqual(capture.stateSequence.workflowOpenAfterChatBackground.lastOpenedThreadId, workflowThread.id)
        XCTAssertTrue(capture.stateSequence.workflowOpenAfterChatBackground.lastSessionOnThread)
        XCTAssertEqual(capture.stateSequence.coldLaunchAfterPollutedWorkflowOpen.restoredSurface, "workflowRun")

        model.showSelectedThread(chatThread)
        model.persistLastSessionLocation()

        XCTAssertEqual(model.persistedLastOpenedThreadId, chatThread.id)
        XCTAssertTrue(model.persistedLastSessionWasOnThread)

        model.showWorkflowRun(
            workflowRunId: workflowThread.workflowRunId ?? workflowThread.id,
            thread: workflowThread,
            invalidatesPendingThreadOpen: true,
            source: .replace
        )

        XCTAssertEqual(
            model.persistedLastOpenedThreadId,
            chatThread.id,
            "Workflow-run surfaces must not overwrite the last-opened thread slot."
        )
        XCTAssertFalse(
            model.persistedLastSessionWasOnThread,
            "Opening a workflow-run surface must make cold-launch restore land on the home list, even before scenePhase background persistence runs."
        )
    }

    func testColdLaunchRestoreRejectsAlreadyPollutedWorkflowRunDefaults() async throws {
        let capture = try Self.simulatorCapture()
        let model = makeModel()
        let workflowThread = capture.gatewayRecords.workflowThread.threadSummary()

        XCTAssertEqual(capture.stateSequence.coldLaunchAfterPollutedWorkflowOpen.lastOpenedThreadId, workflowThread.id)
        XCTAssertTrue(capture.stateSequence.coldLaunchAfterPollutedWorkflowOpen.lastSessionOnThread)

        model.threads = [workflowThread]
        model.restorePersistedLastOpenedThreadId(
            capture.stateSequence.coldLaunchAfterPollutedWorkflowOpen.lastOpenedThreadId
        )
        model.persistLastSessionRestorable(
            capture.stateSequence.coldLaunchAfterPollutedWorkflowOpen.lastSessionOnThread
        )

        await model.restoreLastOpenedThreadIfNeeded()

        XCTAssertNil(model.selectedThread)
        XCTAssertFalse(model.isWorkflowRunSurfaceActive)
        XCTAssertTrue(model.isHomeVisible)
        XCTAssertNil(model.persistedLastOpenedThreadId)
        XCTAssertFalse(model.persistedLastSessionWasOnThread)
        XCTAssertEqual(model.workflowRunPanelState.mode, .idle)
    }

    private func makeModel() -> GaryxMobileModel {
        let suiteName = "GaryxLastOpenedWorkflowRestoreTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        defaults.set("http://127.0.0.1:31337", forKey: GaryxMobileSettingsKeys.gatewayUrl)
        return GaryxMobileModel(defaults: defaults)
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
    var gatewayRecords: GatewayRecords
    var stateSequence: StateSequence

    struct GatewayRecords: Decodable {
        var chatThread: CapturedThreadSummary
        var workflowThread: CapturedThreadSummary
    }

    struct StateSequence: Decodable {
        var chatBackground: PersistedLocation
        var workflowOpenAfterChatBackground: PersistedLocation
        var coldLaunchAfterPollutedWorkflowOpen: RestoredPersistedLocation
    }

    struct PersistedLocation: Decodable {
        var lastOpenedThreadId: String
        var lastSessionOnThread: Bool
    }

    struct RestoredPersistedLocation: Decodable {
        var lastOpenedThreadId: String
        var lastSessionOnThread: Bool
        var restoredSurface: String
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
    var teamId: String?
    var teamName: String?
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
            teamId: teamId,
            teamName: teamName,
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
