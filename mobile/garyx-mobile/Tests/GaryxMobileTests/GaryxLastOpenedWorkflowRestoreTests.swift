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

    func testColdLaunchRestoreRejectsWorkflowRunFetchedAfterRecentListOmission() async throws {
        let capture = try Self.simulatorCapture()
        let model = makeModel()
        let workflowThread = capture.gatewayRecords.workflowThread.threadSummary()
        let workflowThreadBody = Self.threadSummaryJSON(workflowThread)

        model.connectionState = .ready(version: "test")
        model.restorePersistedLastOpenedThreadId(
            capture.stateSequence.coldLaunchAfterPollutedWorkflowOpen.lastOpenedThreadId
        )
        model.persistLastSessionRestorable(
            capture.stateSequence.coldLaunchAfterPollutedWorkflowOpen.lastSessionOnThread
        )

        XCTAssertTrue(model.threads.isEmpty, "The reproduction must start with no workflow thread in memory.")
        XCTAssertFalse(model.threadOpenState.hasPendingIntent)

        URLProtocol.registerClass(GaryxLastOpenedWorkflowRestoreURLProtocol.self)
        defer {
            GaryxLastOpenedWorkflowRestoreURLProtocol.requestHandler = nil
            URLProtocol.unregisterClass(GaryxLastOpenedWorkflowRestoreURLProtocol.self)
        }
        GaryxLastOpenedWorkflowRestoreURLProtocol.requestHandler = { request in
            let url = try XCTUnwrap(request.url)
            let path = URLComponents(url: url, resolvingAgainstBaseURL: false)?.percentEncodedPath ?? url.path
            let body: String
            switch path {
            case "/api/recent-threads":
                body = """
                {
                  "threads": [],
                  "count": 0,
                  "limit": 30,
                  "offset": 0,
                  "total": 0,
                  "has_more": false
                }
                """
            case "/api/thread-pins":
                body = #"{"thread_ids":[],"pins":[]}"#
            case "/api/threads/\(Self.pathSegmentEncoded(workflowThread.id))":
                body = workflowThreadBody
            default:
                let response = HTTPURLResponse(
                    url: url,
                    statusCode: 404,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )!
                return (response, Data(#"{"error":"not found"}"#.utf8))
            }
            let response = HTTPURLResponse(
                url: url,
                statusCode: 200,
                httpVersion: nil,
                headerFields: ["Content-Type": "application/json"]
            )!
            return (response, Data(body.utf8))
        }

        await model.restoreLastOpenedThreadIfNeeded()

        XCTAssertNil(model.selectedThread)
        XCTAssertFalse(
            model.isWorkflowRunSurfaceActive,
            "A workflow-run summary fetched through /api/threads/{id} must not become an automatic cold-start restore target."
        )
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

    private static func threadSummaryJSON(_ thread: GaryxThreadSummary) -> String {
        let fields: [(String, Any?)] = [
            ("thread_id", thread.id),
            ("thread_key", thread.id),
            ("label", thread.title),
            ("created_at", thread.createdAt),
            ("updated_at", thread.updatedAt),
            ("last_message_preview", thread.lastMessagePreview),
            ("workspace_dir", thread.workspacePath),
            ("message_count", thread.messageCount),
            ("agent_id", thread.agentId),
            ("team_id", thread.teamId),
            ("team_name", thread.teamName),
            ("provider_type", thread.providerType),
            ("recent_run_id", thread.recentRunId),
            ("active_run_id", thread.activeRunId),
            ("run_state", thread.runState),
            ("worktree", thread.worktreePath.map { ["path": $0] }),
            ("thread_type", thread.threadType),
            ("thread_kind", thread.threadType),
            ("workflow_run_id", thread.workflowRunId),
            ("exclude_from_recent", true),
        ]
        let object = Dictionary(uniqueKeysWithValues: fields.compactMap { key, value in
            value.map { (key, $0) }
        })
        let data = try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return String(data: data, encoding: .utf8)!
    }

    private static func pathSegmentEncoded(_ value: String) -> String {
        let allowed = CharacterSet(charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
        return value.addingPercentEncoding(withAllowedCharacters: allowed) ?? value
    }
}

private final class GaryxLastOpenedWorkflowRestoreURLProtocol: URLProtocol {
    static var requestHandler: ((URLRequest) throws -> (HTTPURLResponse, Data))?

    override class func canInit(with request: URLRequest) -> Bool {
        true
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        guard let requestHandler = Self.requestHandler else {
            client?.urlProtocol(self, didFailWithError: URLError(.badServerResponse))
            return
        }
        do {
            let (response, data) = try requestHandler(request)
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
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
