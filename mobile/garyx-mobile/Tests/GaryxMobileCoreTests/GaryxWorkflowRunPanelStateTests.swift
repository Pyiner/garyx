import XCTest
@testable import GaryxMobileCore

final class GaryxWorkflowRunPanelStateTests: XCTestCase {
    func testWorkflowProjectionFixtureDecodesWithoutClientRollup() throws {
        let drilldown = try Self.workflowFixture()
        let presentation = drilldown.presentation

        XCTAssertEqual(presentation.workflowRunId, "thread::workflow-1001")
        XCTAssertEqual(presentation.threadId, "thread::workflow-1001")
        XCTAssertEqual(presentation.activePhase?.phaseId, "review")
        XCTAssertEqual(presentation.phaseStatus.map(\.status), ["succeeded", "running", "queued"])
        XCTAssertEqual(presentation.phases.map(\.phaseId), ["plan", "review", "finalize"])
        XCTAssertEqual(presentation.phases[1].children.map(\.workflowChildRunId), ["child::risk", "child::lint"])
        XCTAssertEqual(presentation.childCards.map(\.workflowChildRunId), ["child::risk", "child::lint", "child::summary"])
        XCTAssertEqual(presentation.counts.completed, 2)
        XCTAssertEqual(presentation.eventsSeed.latestSeedEventSeq, 2)
        XCTAssertFalse(presentation.terminalComplete)
        XCTAssertFalse(presentation.stale ?? true)
    }

    func testWorkflowRunPanelStateAppliesSnapshotsMonotonically() throws {
        let drilldown = try Self.workflowFixture()
        var state = GaryxWorkflowRunPanelState()

        state.beginRefresh(workflowRunId: "thread::workflow-1001")
        XCTAssertTrue(state.applyResult(workflowRunId: "thread::workflow-1001", drilldown: drilldown))
        XCTAssertEqual(state.phase, .loaded)
        XCTAssertEqual(state.lastAppliedSnapshotVersion, 1_782_028_950_000)
        XCTAssertEqual(state.presentation?.title, "Release readiness review")

        var stale = drilldown
        stale.presentation.snapshotVersion = 1
        stale.presentation.latestEventSeq = 1
        stale.presentation.title = "Stale title"
        XCTAssertFalse(state.applyResult(workflowRunId: "thread::workflow-1001", drilldown: stale))
        XCTAssertEqual(state.lastAppliedSnapshotVersion, 1_782_028_950_000)
        XCTAssertEqual(state.presentation?.title, "Release readiness review")

        var other = drilldown
        other.presentation.workflowRunId = "thread::other"
        XCTAssertFalse(state.applyResult(workflowRunId: "thread::workflow-1001", drilldown: other))
    }

    func testWorkflowPollPolicyUsesTerminalCompleteAndForeground() throws {
        var drilldown = try Self.workflowFixture()

        XCTAssertTrue(
            GaryxWorkflowRunPollPolicy.policy(
                presentation: drilldown.presentation,
                foregroundVisible: true
            ).shouldPoll
        )
        XCTAssertFalse(
            GaryxWorkflowRunPollPolicy.policy(
                presentation: drilldown.presentation,
                foregroundVisible: false
            ).shouldPoll
        )

        drilldown.presentation.terminalComplete = true
        XCTAssertFalse(
            GaryxWorkflowRunPollPolicy.policy(
                presentation: drilldown.presentation,
                foregroundVisible: true
            ).shouldPoll
        )

        let policy = GaryxWorkflowRunPollPolicy.policy(
            presentation: drilldown.presentation,
            foregroundVisible: true
        )
        XCTAssertFalse(policy.acceptsEvent(seq: 2))
        XCTAssertTrue(policy.acceptsEvent(seq: 3))
    }

    func testWorkflowDestinationUsesThreadSubtype() {
        let workflowThread = GaryxThreadSummary(
            id: "thread::workflow",
            title: "Workflow",
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
            worktreePath: nil,
            threadType: "workflow_run",
            workflowRunId: "thread::workflow"
        )
        let chatThread = GaryxThreadSummary(
            id: "thread::chat",
            title: "Chat",
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
            worktreePath: nil,
            threadType: "runtime"
        )
        let unknownThread = GaryxThreadSummary(
            id: "thread::unknown",
            title: "Unknown",
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

        XCTAssertEqual(
            GaryxWorkflowRunDestination.destination(for: workflowThread),
            .workflowRun(runId: "thread::workflow")
        )
        XCTAssertEqual(
            GaryxWorkflowRunDestination.destination(for: chatThread),
            .chat(threadId: "thread::chat")
        )
        XCTAssertEqual(
            GaryxWorkflowRunDestination.destination(for: unknownThread),
            .unresolved(threadId: "thread::unknown")
        )
        XCTAssertEqual(
            GaryxWorkflowRunDestination.destination(threadId: "thread::missing", summary: nil),
            .unresolved(threadId: "thread::missing")
        )
    }

    func testTaskSummaryDecodesWorkflowExecutorFromNestedEnvelope() throws {
        let summary = try JSONDecoder().decode(
            GaryxTaskSummary.self,
            from: Data(
                """
                {
                  "thread_id": "thread::workflow-1001",
                  "task_id": "#TASK-1001",
                  "task": {
                    "number": 1001,
                    "title": "Run workflow",
                    "status": "in_progress",
                    "executor": {
                      "type": "workflow",
                      "workflow_id": "release-check",
                      "workflow_version": 3
                    }
                  }
                }
                """.utf8
            )
        )

        XCTAssertEqual(summary.id, "#TASK-1001")
        XCTAssertEqual(summary.threadId, "thread::workflow-1001")
        XCTAssertTrue(summary.executor?.isWorkflow == true)
        XCTAssertEqual(summary.executor?.workflowId, "release-check")
        XCTAssertEqual(summary.executor?.workflowVersion, 3)
    }

    func testWorkflowClientMethodsEncodeExpectedPaths() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxWorkflowURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxWorkflowURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        var paths: [String] = []
        GaryxWorkflowURLProtocolStub.requestHandler = { request in
            let url = try XCTUnwrap(request.url)
            paths.append(URLComponents(url: url, resolvingAgainstBaseURL: false)?.percentEncodedPath ?? "")
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: url,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (response, try Self.workflowFixtureData())
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/garyx"))
            ),
            session: session,
            retryPolicy: .disabled
        )

        let workflow = try await client.getWorkflowRun(workflowRunId: "thread::workflow-1001")

        XCTAssertEqual(workflow.presentation.workflowRunId, "thread::workflow-1001")
        XCTAssertEqual(paths, ["/garyx/api/workflows/thread%3A%3Aworkflow-1001"])
    }

    private static func workflowFixture() throws -> GaryxWorkflowRunDrilldown {
        try JSONDecoder().decode(GaryxWorkflowRunDrilldown.self, from: workflowFixtureData())
    }

    private static func workflowFixtureData() throws -> Data {
        let fileURL = URL(fileURLWithPath: #filePath)
        var root = fileURL
        for _ in 0..<5 {
            root.deleteLastPathComponent()
        }
        let fixtureURL = root
            .appendingPathComponent("test-fixtures")
            .appendingPathComponent("workflow-presentation")
            .appendingPathComponent("mobile-desktop-parity.json")
        return try Data(contentsOf: fixtureURL)
    }
}

private final class GaryxWorkflowURLProtocolStub: URLProtocol {
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
