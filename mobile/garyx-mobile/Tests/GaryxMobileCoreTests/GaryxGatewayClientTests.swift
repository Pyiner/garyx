import XCTest
@testable import GaryxMobileCore

final class GaryxGatewayClientTests: XCTestCase {
    func testEmptyGatewayURLErrorIsActionable() {
        XCTAssertEqual(
            GaryxGatewayError.invalidURL("").errorDescription,
            "Enter the Garyx gateway URL from the Mac app."
        )
    }

    func testInvalidGatewayURLErrorTrimsInput() {
        XCTAssertEqual(
            GaryxGatewayError.invalidURL("  http://  ").errorDescription,
            "Invalid Garyx gateway URL: http://"
        )
    }

    func testHTTPStatusErrorExtractsGatewayMessage() {
        XCTAssertEqual(
            GaryxGatewayError.httpStatus(404, #"{"error":"thread not found"}"#).errorDescription,
            "thread not found"
        )
    }

    func testHTTPStatusErrorExtractsNestedProviderAuthMessage() {
        XCTAssertEqual(
            GaryxGatewayError.httpStatus(
                504,
                #"{"error":{"code":"claude_auth_start_timeout","message":"Timed out waiting for Claude Code login URL."}}"#
            ).errorDescription,
            "Timed out waiting for Claude Code login URL."
        )
    }

    func testPathSegmentEncodingEscapesSlash() {
        XCTAssertEqual(
            GaryxGatewayClient.encodePathSegment("thread/with/slash"),
            "thread%2Fwith%2Fslash"
        )
        XCTAssertEqual(
            GaryxGatewayClient.encodePathSegment("thread::a&b"),
            "thread%3A%3Aa%26b"
        )
    }

    func testThreadStreamRequestEncodesReplayWindowParameters() throws {
        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: URL(string: "http://127.0.0.1:31337/garyx")!,
                authToken: "test-token"
            )
        )

        let request = try client.threadStreamRequest(
            threadId: "thread::test/child",
            afterSeq: 9,
            replayScope: .initial,
            initialUserTurns: 1,
            renderFloor: 7
        )
        let components = try XCTUnwrap(URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false))
        let queryItems = components.queryItems ?? []

        XCTAssertEqual(components.percentEncodedPath, "/garyx/api/threads/thread%3A%3Atest%2Fchild/stream")
        XCTAssertEqual(queryItems.first(where: { $0.name == "after_seq" })?.value, "9")
        XCTAssertEqual(queryItems.first(where: { $0.name == "replay_scope" })?.value, "initial")
        XCTAssertEqual(queryItems.first(where: { $0.name == "initial_user_turns" })?.value, "1")
        XCTAssertEqual(queryItems.first(where: { $0.name == "render_floor" })?.value, "7")
        // #TASK-1956 batch 3: every stream connection declares delta mode;
        // the frame processor reassembles full snapshots downstream.
        XCTAssertEqual(queryItems.first(where: { $0.name == "render_mode" })?.value, "delta")
        XCTAssertEqual(request.value(forHTTPHeaderField: "Accept"), "text/event-stream")
    }

    func testArchiveThreadRequestEncodesEndpointKeys() throws {
        let request = GaryxArchiveThreadRequest(endpointKeys: [
            "telegram::main::1000000001",
            "api::main::loop"
        ])

        let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any]

        XCTAssertEqual(
            object?["endpointKeys"] as? [String],
            ["telegram::main::1000000001", "api::main::loop"]
        )
    }

    func testBuiltInAvatarStyleCatalogHasEightOptions() {
        let styles = GaryxAvatarStyleOption.builtIn
        XCTAssertEqual(styles.count, 8)
        XCTAssertEqual(Set(styles.map(\.id)).count, 8)
        XCTAssertEqual(GaryxAvatarStyleOption.defaultId, "clean_glyph")
        XCTAssertTrue(styles.allSatisfy { !$0.label.isEmpty && !$0.prompt.isEmpty })
    }

    func testAvatarPromptBuilderMatchesAgentShape() {
        let prompt = GaryxAvatarPromptBuilder.prompt(
            displayName: "Planning Agent",
            identifier: "planning-agent",
            stylePrompt: "layered paper-cut icon"
        )

        XCTAssertTrue(prompt.contains(#"AI agent named "Planning Agent""#))
        XCTAssertTrue(prompt.contains("Visual style: layered paper-cut icon."))
        XCTAssertTrue(prompt.contains("one centered abstract agent mark"))
        XCTAssertTrue(prompt.contains("Do not include text"))
    }

    func testMobileConnectLinkRoundTripsGatewaySettings() throws {
        let url = try XCTUnwrap(
            GaryxMobileConnectLink.make(
                gatewayUrl: "http://192.168.1.20:31337",
                gatewayAuthToken: "test gateway token",
                gatewayHeaders: "X-Garyx-Proxy: proxy-token\nX-Trace-Id=trace-123"
            )
        )

        let payload = try XCTUnwrap(GaryxMobileConnectLink.parse(url))

        XCTAssertEqual(payload.gatewayUrl, "http://192.168.1.20:31337")
        XCTAssertEqual(payload.gatewayAuthToken, "test gateway token")
        XCTAssertEqual(payload.gatewayHeaders, "X-Garyx-Proxy: proxy-token\nX-Trace-Id=trace-123")
    }

    func testGatewayHeadersParseMultiLineBlock() {
        XCTAssertEqual(
            GaryxGatewayHeaders.parse(
                """
                X-Garyx-Proxy: proxy-token
                X-Trace-Id=trace-123
                # ignored
                invalid header: skipped
                """
            ),
            [
                "X-Garyx-Proxy": "proxy-token",
                "X-Trace-Id": "trace-123",
            ]
        )
    }

    func testMobileConnectLinkAcceptsTokenAlias() throws {
        let url = try XCTUnwrap(
            URL(string: "garyx://connect?url=http%3A%2F%2F192.168.1.20%3A31337&token=test-token")
        )

        let payload = try XCTUnwrap(GaryxMobileConnectLink.parse(url))

        XCTAssertEqual(payload.gatewayUrl, "http://192.168.1.20:31337")
        XCTAssertEqual(payload.gatewayAuthToken, "test-token")
    }

    func testStreamInputRequestEncodesGatewayShape() throws {
        let request = GaryxStreamInputRequest(
            threadId: "thread::test",
            clientIntentId: "intent-test",
            message: "follow up",
            attachments: [
                GaryxPromptAttachment(
                    kind: "file",
                    path: "/workspace/project/note.md",
                    name: "note.md",
                    mediaType: "text/markdown"
                ),
            ]
        )

        let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any]

        XCTAssertEqual(object?["threadId"] as? String, "thread::test")
        XCTAssertEqual(object?["clientIntentId"] as? String, "intent-test")
        XCTAssertEqual(object?["message"] as? String, "follow up")
        let attachments = try XCTUnwrap(object?["attachments"] as? [[String: Any]])
        XCTAssertEqual(attachments.first?["path"] as? String, "/workspace/project/note.md")
    }

    func testAgentRequestEncodesExpectedUpdatedAtToken() throws {
        let agentRequest = GaryxCustomAgentRequest(
            agentId: "agent-test",
            displayName: "Agent Test",
            providerType: "codex_app_server",
            expectedUpdatedAt: "2026-01-01T00:00:00Z"
        )
        let agentObject = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(agentRequest)) as? [String: Any]
        XCTAssertEqual(agentObject?["expected_updated_at"] as? String, "2026-01-01T00:00:00Z")

        // Create requests omit the token entirely instead of sending null.
        let createRequest = GaryxCustomAgentRequest(
            agentId: "agent-test",
            displayName: "Agent Test",
            providerType: "codex_app_server"
        )
        let createObject = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(createRequest)) as? [String: Any]
        XCTAssertNil(createObject?["expected_updated_at"])
    }

    func testGetAgentUsesAuthoritativeSingleAgentRoute() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        GaryxURLProtocolStub.requestHandler = { request in
            XCTAssertEqual(request.httpMethod, "GET")
            XCTAssertEqual(
                request.url.flatMap { URLComponents(url: $0, resolvingAgainstBaseURL: false) }?.percentEncodedPath,
                "/api/custom-agents/agent%2Ftest"
            )
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (
                response,
                Data(
                    #"{"agent_id":"agent/test","display_name":"Authoritative Agent","provider_type":"codex_app_server","model":"test-model","model_service_tier":"priority","provider_env":{"KEEP":"value"},"updated_at":"2026-07-13T12:00:00Z"}"#.utf8
                )
            )
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: .disabled
        )

        let agent = try await client.getAgent(agentId: "agent/test")
        XCTAssertEqual(agent.id, "agent/test")
        XCTAssertEqual(agent.displayName, "Authoritative Agent")
        XCTAssertEqual(agent.modelServiceTier, "priority")
        XCTAssertEqual(agent.providerEnv, ["KEEP": "value"])
        XCTAssertEqual(agent.updatedAt, "2026-07-13T12:00:00Z")
    }

    func testAgentCatalogToggleAndDefaultUseAvailabilityRoutes() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        let requestCount = GaryxAtomicCounter()
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        GaryxURLProtocolStub.requestHandler = { request in
            let call = requestCount.increment()
            let path = request.url.flatMap {
                URLComponents(url: $0, resolvingAgainstBaseURL: false)
            }?.percentEncodedPath
            let body: String
            switch call {
            case 1:
                XCTAssertEqual(request.httpMethod, "GET")
                XCTAssertEqual(path, "/api/custom-agents")
                body = #"{"agents":[{"agent_id":"codex","enabled":true}],"default_agent_id":"claude","effective_default_agent_id":"codex"}"#
            case 2:
                XCTAssertEqual(request.httpMethod, "PATCH")
                XCTAssertEqual(path, "/api/custom-agents/codex/toggle")
                let object = try XCTUnwrap(
                    JSONSerialization.jsonObject(with: try XCTUnwrap(garyxRequestBodyData(from: request)))
                        as? [String: Any]
                )
                XCTAssertEqual(object["enabled"] as? Bool, false)
                body = #"{"agent_id":"codex","enabled":false}"#
            case 3:
                XCTAssertEqual(request.httpMethod, "PATCH")
                XCTAssertEqual(path, "/api/custom-agents/codex/default")
                body = #"{"agent_id":"codex","enabled":true}"#
            default:
                XCTFail("Unexpected request \(call)")
                body = #"{}"#
            }
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (response, Data(body.utf8))
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: .disabled
        )

        let catalog = try await client.listAgentCatalog()
        XCTAssertEqual(catalog.defaultAgentId, "claude")
        XCTAssertEqual(catalog.effectiveDefaultAgentId, "codex")
        XCTAssertEqual(catalog.agents.first?.enabled, true)
        let disabled = try await client.setAgentEnabled(agentId: "codex", enabled: false)
        let defaultAgent = try await client.setDefaultAgent(agentId: "codex")
        XCTAssertEqual(disabled.enabled, false)
        XCTAssertEqual(defaultAgent.id, "codex")
        XCTAssertEqual(requestCount.value(), 3)
    }

    func testCustomAgentRequestEncodesEmptyModelAsPresentValue() throws {
        let request = GaryxCustomAgentRequest(
            agentId: "agent-test",
            displayName: "Agent Test",
            providerType: "codex_app_server",
            model: "",
            modelReasoningEffort: "",
            modelServiceTier: "",
            systemPrompt: "Use synthetic instructions."
        )

        let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any]

        XCTAssertEqual(object?["model"] as? String, "")
        XCTAssertEqual(object?["model_reasoning_effort"] as? String, "")
        XCTAssertEqual(object?["model_service_tier"] as? String, "")
    }

    func testCustomAgentRequestEncodesEmptySystemPromptAsPresentValue() throws {
        let request = GaryxCustomAgentRequest(
            agentId: "agent-test",
            displayName: "Agent Test",
            providerType: "claude_code",
            systemPrompt: ""
        )

        let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any]

        XCTAssertEqual(object?["system_prompt"] as? String, "")
    }

    func testStartChatRequestEncodesGatewayShape() throws {
        let request = GaryxStartChatRequest(
            threadId: "thread::test",
            message: "hello",
            attachments: [
                GaryxPromptAttachment(
                    kind: "image",
                    path: "/workspace/project/image.png",
                    name: "image.png",
                    mediaType: "image/png"
                ),
            ],
            workspacePath: "/workspace/project",
            metadata: [
                "client": "garyx-mobile",
                "client_intent_id": "intent-test",
            ]
        )

        let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any]

        XCTAssertEqual(object?["threadId"] as? String, "thread::test")
        XCTAssertEqual(object?["message"] as? String, "hello")
        XCTAssertEqual(object?["fromId"] as? String, "garyx-mobile")
        XCTAssertEqual(object?["accountId"] as? String, "main")
        XCTAssertEqual(object?["waitForResponse"] as? Bool, false)
        XCTAssertEqual(object?["workspacePath"] as? String, "/workspace/project")
        XCTAssertEqual((object?["metadata"] as? [String: String])?["client"], "garyx-mobile")
        let attachments = try XCTUnwrap(object?["attachments"] as? [[String: Any]])
        XCTAssertEqual(attachments.first?["media_type"] as? String, "image/png")
    }

    func testStartChatResultDecodesGatewayShape() throws {
        let result = try JSONDecoder().decode(
            GaryxStartChatResult.self,
            from: Data(
                """
                {
                  "status": "accepted",
                  "run_id": "run-test",
                  "thread_id": "thread::test"
                }
                """.utf8
            )
        )

        XCTAssertEqual(result.status, "accepted")
        XCTAssertEqual(result.runId, "run-test")
        XCTAssertEqual(result.threadId, "thread::test")
    }

    func testStreamInputResultDecodesGatewayShape() throws {
        let result = try JSONDecoder().decode(
            GaryxStreamInputResult.self,
            from: Data(
                """
                {
                  "status": "queued",
                  "thread_status": "queued",
                  "client_intent_id": "intent-test",
                  "pending_input_id": "pending-test",
                  "thread_id": "thread::test"
                }
                """.utf8
            )
        )

        XCTAssertEqual(result.status, "queued")
        XCTAssertEqual(result.threadStatus, "queued")
        XCTAssertEqual(result.clientIntentId, "intent-test")
        XCTAssertEqual(result.pendingInputId, "pending-test")
        XCTAssertEqual(result.threadId, "thread::test")
    }

    func testThreadSummaryDecodesGatewaySnakeCase() throws {
        let data = Data(
            """
            {
              "thread_id": "thread::test",
              "label": "Mobile thread",
              "workspace_dir": "/path/to/repo",
              "message_count": 3,
              "last_user_message": "ship it",
              "agent_id": "claude",
              "provider_type": "claude_code",
              "recent_run_id": "run-test",
              "active_run_id": "run-active",
              "worktree": {
                "worktree_dir": "/workspace/.garyx/worktrees/thread-test"
              }
            }
            """.utf8
        )

        let summary = try JSONDecoder().decode(GaryxThreadSummary.self, from: data)

        XCTAssertEqual(summary.id, "thread::test")
        XCTAssertEqual(summary.title, "Mobile thread")
        XCTAssertEqual(summary.workspacePath, "/path/to/repo")
        XCTAssertEqual(summary.messageCount, 3)
        XCTAssertEqual(summary.lastMessagePreview, "ship it")
        XCTAssertEqual(summary.agentId, "claude")
        XCTAssertEqual(summary.providerType, "claude_code")
        XCTAssertEqual(summary.recentRunId, "run-test")
        XCTAssertEqual(summary.activeRunId, "run-active")
        XCTAssertEqual(summary.worktreePath, "/workspace/.garyx/worktrees/thread-test")
    }

    func testThreadSummaryUsesNewThreadPlaceholderWhenUnlabeled() throws {
        let summary = try JSONDecoder().decode(
            GaryxThreadSummary.self,
            from: Data(
                """
                {
                  "thread_id": "thread::unlabeled"
                }
                """.utf8
            )
        )

        XCTAssertEqual(summary.id, "thread::unlabeled")
        XCTAssertEqual(summary.title, "New Thread")
    }

    func testThreadPinsPageDecodesGatewayShape() throws {
        let page = try JSONDecoder().decode(
            GaryxThreadPinsPage.self,
            from: Data(
                """
                {
                  "thread_ids": ["thread::one", " thread::two ", "thread::one", ""],
                  "revision": 7
                }
                """.utf8
            )
        )

        XCTAssertEqual(page.threadIds, ["thread::one", "thread::two"])
        XCTAssertEqual(page.revision, 7)
    }

    func testThreadPinsPageDecodesPinsFallback() throws {
        let page = try JSONDecoder().decode(
            GaryxThreadPinsPage.self,
            from: Data(
                """
                {
                  "pins": [
                    { "thread_id": "thread::from-snake", "pinned_at": "2026-05-22T00:00:00.000Z" },
                    { "threadId": "thread::from-camel", "pinned_at": "2026-05-22T00:00:01.000Z" }
                  ]
                }
                """.utf8
            )
        )

        XCTAssertEqual(page.threadIds, ["thread::from-snake", "thread::from-camel"])
        XCTAssertEqual(page.revision, 0)
    }

    func testReorderThreadPinsSendsOneCASAttemptAndDecodesAcceptedPage() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        GaryxURLProtocolStub.requestHandler = { request in
            XCTAssertEqual(request.httpMethod, "PUT")
            XCTAssertEqual(request.url?.path, "/garyx/api/thread-pins")
            let body = try XCTUnwrap(garyxRequestBodyData(from: request))
            let object = try XCTUnwrap(
                JSONSerialization.jsonObject(with: body) as? [String: Any]
            )
            XCTAssertEqual(object["thread_ids"] as? [String], ["thread::two", "thread::one"])
            XCTAssertEqual(object["expected_revision"] as? Int, 8)
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (
                response,
                Data(#"{"thread_ids":["thread::two","thread::one"],"revision":9}"#.utf8)
            )
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: URL(string: "http://127.0.0.1:31337/garyx")!
            ),
            session: session
        )

        let result = try await client.reorderThreadPins(
            threadIds: ["thread::two", "thread::one"],
            expectedRevision: 8
        )

        XCTAssertEqual(
            result,
            .accepted(GaryxThreadPinsPage(
                threadIds: ["thread::two", "thread::one"],
                revision: 9
            ))
        )
    }

    func testReorderThreadPinsReturnsConflictPageWithoutInternalRetry() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        var requestCount = 0
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        GaryxURLProtocolStub.requestHandler = { request in
            requestCount += 1
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 409,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (
                response,
                Data(#"{"thread_ids":["thread::one","thread::two"],"revision":11}"#.utf8)
            )
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: URL(string: "http://127.0.0.1:31337/garyx")!
            ),
            session: session
        )

        let result = try await client.reorderThreadPins(
            threadIds: ["thread::two", "thread::one"],
            expectedRevision: 8
        )

        XCTAssertEqual(requestCount, 1)
        XCTAssertEqual(
            result,
            .conflict(GaryxThreadPinsPage(
                threadIds: ["thread::one", "thread::two"],
                revision: 11
            ))
        )
    }

    func testRecentThreadsPageDecodesGatewayShape() throws {
        let page = try JSONDecoder().decode(
            GaryxRecentThreadsPage.self,
            from: Data(
                """
                {
                  "threads": [
                    {
                      "thread_id": "thread::recent",
                      "title": "Recent Thread",
                      "workspace_dir": "/workspace/project",
                      "message_count": 5,
                      "last_message_preview": "latest user message",
                      "active_run_id": "run::active",
                      "run_state": "running",
                      "last_active_at": "2026-05-23T10:00:00.000Z",
                      "activity_seq": 42
                    }
                  ],
                  "count": 1,
                  "limit": 80,
                  "total": 42,
                  "has_more": true,
                  "next_cursor": "cursor-42",
                  "store_incarnation_id": "11111111-1111-4111-8111-111111111111",
                  "server_boot_id": "22222222-2222-4222-8222-222222222222"
                }
                """.utf8
            )
        )

        XCTAssertEqual(page.count, 1)
        XCTAssertEqual(page.limit, 80)
        XCTAssertEqual(page.total, 42)
        XCTAssertTrue(page.hasMore)
        XCTAssertEqual(page.nextCursor, "cursor-42")
        XCTAssertEqual(page.storeIncarnationId, "11111111-1111-4111-8111-111111111111")
        XCTAssertEqual(page.serverBootId, "22222222-2222-4222-8222-222222222222")
        XCTAssertEqual(page.threads.first?.activitySeq, 42)
        XCTAssertEqual(page.threads.first?.id, "thread::recent")
        XCTAssertEqual(page.threads.first?.title, "Recent Thread")
        XCTAssertEqual(page.threads.first?.workspacePath, "/workspace/project")
        XCTAssertEqual(page.threads.first?.lastMessagePreview, "latest user message")
        XCTAssertEqual(page.threads.first?.activeRunId, "run::active")
        XCTAssertEqual(page.threads.first?.runState, "running")
        XCTAssertEqual(page.threads.first?.updatedAt, "2026-05-23T10:00:00.000Z")
    }

    func testListRecentThreadsDefaultsToAllAndThirty() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        GaryxURLProtocolStub.requestHandler = { request in
            XCTAssertEqual(request.httpMethod, "GET")
            XCTAssertEqual(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)?.percentEncodedPath,
                "/garyx/api/recent-threads"
            )
            let queryItems = URLComponents(
                url: try XCTUnwrap(request.url),
                resolvingAgainstBaseURL: false
            )?.queryItems ?? []
            XCTAssertEqual(queryItems.first(where: { $0.name == "tasks" })?.value, "include")
            XCTAssertEqual(queryItems.first(where: { $0.name == "limit" })?.value, "30")
            XCTAssertNil(queryItems.first(where: { $0.name == "cursor" }))
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (
                response,
                Data(
                    """
                    {
                      "threads": [],
                      "count": 0,
                      "limit": 30,
                      "total": 0,
                      "has_more": false,
                      "next_cursor": null,
                      "store_incarnation_id": "11111111-1111-4111-8111-111111111111",
                      "server_boot_id": "22222222-2222-4222-8222-222222222222"
                    }
                    """.utf8
                )
            )
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/garyx"))
            ),
            session: session
        )

        let page = try await client.listRecentThreads()

        XCTAssertEqual(page.limit, 30)
        XCTAssertEqual(page.count, 0)
        XCTAssertFalse(page.hasMore)
    }

    func testListRecentThreadsSendsChatsFilterExplicitly() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        GaryxURLProtocolStub.requestHandler = { request in
            let queryItems = URLComponents(
                url: try XCTUnwrap(request.url),
                resolvingAgainstBaseURL: false
            )?.queryItems ?? []
            XCTAssertEqual(queryItems.first(where: { $0.name == "tasks" })?.value, "exclude")
            XCTAssertEqual(queryItems.first(where: { $0.name == "limit" })?.value, "80")
            XCTAssertEqual(queryItems.first(where: { $0.name == "cursor" })?.value, "cursor-20")
            let response = try XCTUnwrap(HTTPURLResponse(
                url: try XCTUnwrap(request.url),
                statusCode: 200,
                httpVersion: nil,
                headerFields: ["Content-Type": "application/json"]
            ))
            return (
                response,
                Data(#"{"threads":[],"count":0,"limit":80,"total":0,"has_more":false,"next_cursor":null,"store_incarnation_id":"11111111-1111-4111-8111-111111111111","server_boot_id":"22222222-2222-4222-8222-222222222222"}"#.utf8)
            )
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/garyx"))
            ),
            session: session
        )

        _ = try await client.listRecentThreads(filter: .nonTask, limit: 80, cursor: "cursor-20")
    }

    func testThreadFavoriteMutationClassificationMatrixUsesOneAttempt() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }
        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/garyx"))
            ),
            session: session,
            retryPolicy: GaryxGatewayRetryPolicy(maxAttempts: 4, initialDelay: 0, jitter: 0)
        )
        let pageFields = """
        "store_incarnation_id":"11111111-1111-4111-8111-111111111111",
        "server_boot_id":"22222222-2222-4222-8222-222222222222",
        "revision":2,
        "thread_ids":["thread::test"],
        "favorites":[{"thread_id":"thread::test","favorited_at":"2026-07-16T00:00:00Z"}]
        """
        let cases: [(String, Int, String, String)] = [
            ("ok", 200, "{\(pageFields)}", "ok"),
            (
                "endpoint tagged",
                409,
                "{\(pageFields),\"kind\":\"garyx_api_error\",\"operation\":\"thread_favorites_put\",\"code\":\"conflict\"}",
                "definitive"
            ),
            (
                "gateway auth tagged",
                401,
                "{\"kind\":\"garyx_api_error\",\"operation\":\"gateway_auth\",\"code\":\"unauthorized\"}",
                "definitive"
            ),
            (
                "wrong operation",
                409,
                "{\(pageFields),\"kind\":\"garyx_api_error\",\"operation\":\"thread_favorites_delete\",\"code\":\"conflict\"}",
                "ambiguous"
            ),
            ("proxy json", 502, #"{"error":"bad gateway"}"#, "ambiguous"),
            ("success decode failure", 200, "{truncated", "ambiguous"),
            ("success contract decode failure", 200, #"{"revision":2}"#, "ambiguous"),
        ]

        for (name, status, body, expected) in cases {
            let attempts = GaryxAtomicCounter()
            GaryxURLProtocolStub.requestHandler = { request in
                _ = attempts.increment()
                XCTAssertEqual(request.httpMethod, "PUT", name)
                let query = URLComponents(
                    url: try XCTUnwrap(request.url),
                    resolvingAgainstBaseURL: false
                )?.queryItems ?? []
                XCTAssertEqual(
                    query.first(where: { $0.name == "expected_revision" })?.value,
                    "1",
                    name
                )
                XCTAssertEqual(
                    query.first(where: { $0.name == "expected_store_incarnation" })?.value,
                    "11111111-1111-4111-8111-111111111111",
                    name
                )
                let response = try XCTUnwrap(
                    HTTPURLResponse(
                        url: try XCTUnwrap(request.url),
                        statusCode: status,
                        httpVersion: nil,
                        headerFields: ["Content-Type": "application/json"]
                    )
                )
                return (response, Data(body.utf8))
            }
            let result = await client.setThreadFavorite(
                threadId: "thread::test",
                favorited: true,
                expectedRevision: 1,
                expectedStoreIncarnation: "11111111-1111-4111-8111-111111111111"
            )
            switch (expected, result) {
            case ("ok", .ok(let page)):
                XCTAssertEqual(page.revision, 2, name)
            case ("definitive", .definitiveEndpointResponse(let response)):
                XCTAssertEqual(response.status, status, name)
                XCTAssertEqual(response.error.kind, "garyx_api_error", name)
            case ("ambiguous", .ambiguous(let response)):
                XCTAssertEqual(response.status, status, name)
            default:
                XCTFail("Unexpected mutation result for \(name): \(result)")
            }
            XCTAssertEqual(attempts.value(), 1, name)
        }
    }

    func testThreadFavoritesReadsDecodeIdentityMembershipAndSnapshotAtomically() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        let requestCounter = GaryxAtomicCounter()
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }
        let pageFields = """
        "store_incarnation_id":"11111111-1111-4111-8111-111111111111",
        "server_boot_id":"22222222-2222-4222-8222-222222222222",
        "revision":7,
        "thread_ids":["thread::favorite"],
        "favorites":[{"thread_id":"thread::favorite","favorited_at":"2026-07-16T08:00:00Z"}]
        """
        GaryxURLProtocolStub.requestHandler = { request in
            let requestIndex = requestCounter.increment()
            XCTAssertEqual(request.httpMethod, "GET")
            let path = try XCTUnwrap(
                URLComponents(
                    url: try XCTUnwrap(request.url),
                    resolvingAgainstBaseURL: false
                )?.percentEncodedPath
            )
            let body: String
            switch requestIndex {
            case 1:
                XCTAssertEqual(path, "/garyx/api/thread-favorites")
                body = "{\(pageFields)}"
            case 2:
                XCTAssertEqual(path, "/garyx/api/thread-favorites/snapshot")
                body = """
                {
                  \(pageFields),
                  "recent": {
                    "threads": [{
                      "thread_id": "thread::favorite",
                      "title": "Favorite thread",
                      "last_active_at": "2026-07-16T08:00:00Z",
                      "last_message_preview": "Latest message",
                      "activity_seq": 51
                    }],
                    "total": 1,
                    "truncated": false
                  }
                }
                """
            default:
                XCTFail("Unexpected favorites request \(requestIndex)")
                throw URLError(.badServerResponse)
            }
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (response, Data(body.utf8))
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/garyx"))
            ),
            session: session,
            retryPolicy: .disabled
        )

        let page = try await client.listThreadFavorites()
        let snapshot = try await client.threadFavoritesSnapshot()

        XCTAssertEqual(page.revision, 7)
        XCTAssertEqual(page.threadIds, ["thread::favorite"])
        XCTAssertEqual(page.favorites.first?.favoritedAt, "2026-07-16T08:00:00Z")
        XCTAssertEqual(snapshot.storeIncarnationId, page.storeIncarnationId)
        XCTAssertEqual(snapshot.serverBootId, page.serverBootId)
        XCTAssertEqual(snapshot.recent.threads.first?.activitySeq, 51)
        XCTAssertFalse(snapshot.recent.truncated)
        XCTAssertEqual(requestCounter.value(), 2)
    }

    func testThreadFavoritesPageRejectsTornMembership() throws {
        XCTAssertThrowsError(
            try JSONDecoder().decode(
                GaryxThreadFavoritesPage.self,
                from: Data(
                    """
                    {
                      "store_incarnation_id": "11111111-1111-4111-8111-111111111111",
                      "server_boot_id": "22222222-2222-4222-8222-222222222222",
                      "revision": 3,
                      "thread_ids": ["thread::one"],
                      "favorites": [{
                        "thread_id": "thread::other",
                        "favorited_at": "2026-07-16T08:00:00Z"
                      }]
                    }
                    """.utf8
                )
            )
        )
    }

    func testThreadFavoriteMutationDistinguishesNotSentFromAmbiguous() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }
        let attempts = GaryxAtomicCounter()
        GaryxURLProtocolStub.requestHandler = { _ in
            _ = attempts.increment()
            throw URLError(.networkConnectionLost)
        }
        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: GaryxGatewayRetryPolicy(maxAttempts: 4, initialDelay: 0, jitter: 0)
        )

        let notSent = await client.setThreadFavorite(
            threadId: "thread::test",
            favorited: true,
            expectedRevision: -1,
            expectedStoreIncarnation: "11111111-1111-4111-8111-111111111111"
        )
        guard case .notSent = notSent else {
            return XCTFail("Invalid precondition must be notSent")
        }
        XCTAssertEqual(attempts.value(), 0)

        let ambiguous = await client.setThreadFavorite(
            threadId: "thread::test",
            favorited: true,
            expectedRevision: 1,
            expectedStoreIncarnation: "11111111-1111-4111-8111-111111111111"
        )
        guard case .ambiguous = ambiguous else {
            return XCTFail("Post-dispatch network loss must be ambiguous")
        }
        XCTAssertEqual(attempts.value(), 1)
    }

    func testPatchArchiveAndDeleteEachUseOneMutationAttempt() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }
        let attempts = GaryxAtomicCounter()
        GaryxURLProtocolStub.requestHandler = { request in
            _ = attempts.increment()
            XCTAssertTrue(["PATCH", "POST", "DELETE"].contains(request.httpMethod ?? ""))
            throw URLError(.networkConnectionLost)
        }
        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: GaryxGatewayRetryPolicy(maxAttempts: 4, initialDelay: 0, jitter: 0)
        )

        do { _ = try await client.updateThread(threadId: "thread::patch", label: "Next") } catch {}
        XCTAssertEqual(attempts.value(), 1)
        do { _ = try await client.archiveThread(threadId: "thread::archive") } catch {}
        XCTAssertEqual(attempts.value(), 2)
        do { _ = try await client.deleteThread(threadId: "thread::delete") } catch {}
        XCTAssertEqual(attempts.value(), 3)
    }

    func testArchiveThreadPostsArchiveRoute() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        GaryxURLProtocolStub.requestHandler = { request in
            XCTAssertEqual(request.httpMethod, "POST")
            XCTAssertEqual(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)?.percentEncodedPath,
                "/garyx/api/threads/thread%3A%3Aarchive%2Fa/archive"
            )
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (
                response,
                Data(
                    """
                    {
                      "archived": true,
                      "deleted": true,
                      "thread_id": "thread::archive/a",
                      "detached_endpoint_keys": ["telegram::main::1000000001"]
                    }
                    """.utf8
                )
            )
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/garyx/"))
            ),
            session: session,
            retryPolicy: .disabled
        )

        let result = try await client.archiveThread(
            threadId: "thread::archive/a",
            endpointKeys: ["telegram::main::1000000001"]
        )

        XCTAssertEqual(result.archived, true)
        XCTAssertEqual(result.deleted, true)
        XCTAssertEqual(result.threadId, "thread::archive/a")
        XCTAssertEqual(result.detachedEndpointKeys, ["telegram::main::1000000001"])
    }

    func testClaudeCodeAuthClientUsesProviderAuthRoutes() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        let requestCounter = GaryxAtomicCounter()
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        GaryxURLProtocolStub.requestHandler = { request in
            let requestIndex = requestCounter.increment()
            let components = try XCTUnwrap(URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false))
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer gateway-token")

            switch requestIndex {
            case 1:
                XCTAssertEqual(request.httpMethod, "POST")
                XCTAssertEqual(components.percentEncodedPath, "/garyx/api/providers/claude_code/auth/start")
                XCTAssertEqual(request.timeoutInterval, 35)
                let body = try XCTUnwrap(garyxRequestBodyData(from: request))
                let object = try XCTUnwrap(JSONSerialization.jsonObject(with: body) as? [String: Any])
                XCTAssertEqual(object["mode"] as? String, "console")
                XCTAssertEqual(object["sso"] as? Bool, true)
                XCTAssertNil(object["email"], "iOS must never send an email in the start request")
                let response = try XCTUnwrap(
                    HTTPURLResponse(
                        url: try XCTUnwrap(request.url),
                        statusCode: 201,
                        httpVersion: nil,
                        headerFields: ["Content-Type": "application/json"]
                    )
                )
                return (
                    response,
                    Data(
                        """
                        {
                          "login_id": "login/with slash",
                          "status": "waiting_for_code",
                          "url": "https://claude.example.test/oauth"
                        }
                        """.utf8
                    )
                )
            case 2:
                XCTAssertEqual(request.httpMethod, "POST")
                XCTAssertEqual(
                    components.percentEncodedPath,
                    "/garyx/api/providers/claude_code/auth/login%2Fwith%20slash/submit"
                )
                let body = try XCTUnwrap(garyxRequestBodyData(from: request))
                let object = try XCTUnwrap(JSONSerialization.jsonObject(with: body) as? [String: Any])
                XCTAssertEqual(object["code"] as? String, "code-test")
                let response = try XCTUnwrap(
                    HTTPURLResponse(
                        url: try XCTUnwrap(request.url),
                        statusCode: 200,
                        httpVersion: nil,
                        headerFields: ["Content-Type": "application/json"]
                    )
                )
                return (
                    response,
                    Data(
                        """
                        {
                          "login_id": "login/with slash",
                          "status": "submitted"
                        }
                        """.utf8
                    )
                )
            case 3:
                XCTAssertEqual(request.httpMethod, "GET")
                XCTAssertEqual(
                    components.percentEncodedPath,
                    "/garyx/api/providers/claude_code/auth/login%2Fwith%20slash"
                )
                let response = try XCTUnwrap(
                    HTTPURLResponse(
                        url: try XCTUnwrap(request.url),
                        statusCode: 200,
                        httpVersion: nil,
                        headerFields: ["Content-Type": "application/json"]
                    )
                )
                return (
                    response,
                    Data(
                        """
                        {
                          "login_id": "login/with slash",
                          "status": "succeeded",
                          "auth_status": {
                            "loggedIn": true,
                            "orgName": "Test Org",
                            "subscriptionType": "team"
                          }
                        }
                        """.utf8
                    )
                )
            default:
                XCTFail("unexpected request \(requestIndex)")
                throw URLError(.badServerResponse)
            }
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/garyx")),
                authToken: "gateway-token"
            ),
            session: session,
            retryPolicy: .disabled
        )

        let start = try await client.startClaudeCodeAuth(
            GaryxClaudeCodeAuthStartRequest(
                mode: .console,
                sso: true
            )
        )
        let submitted = try await client.submitClaudeCodeAuth(
            loginId: start.loginId,
            code: " code-test "
        )
        let status = try await client.claudeCodeAuth(loginId: start.loginId)

        XCTAssertEqual(start.status, .waitingForCode)
        XCTAssertEqual(submitted.status, .submitted)
        XCTAssertEqual(status.status, .succeeded)
        XCTAssertEqual(requestCounter.value(), 3)
    }

    func testRecentThreadsPageRejectsLegacyOffsetShapeWithoutCursorIdentity() throws {
        XCTAssertThrowsError(
            try JSONDecoder().decode(
                GaryxRecentThreadsPage.self,
                from: Data(
                """
                {
                  "threads": [
                    {
                      "thread_id": "thread::legacy-preview",
                      "label": "Legacy Preview",
                      "lastMessagePreview": "legacy preview"
                    }
                  ],
                  "count": 1,
                  "limit": 80
                }
                """.utf8
                )
            )
        )
    }

    func testMobileDashboardPayloadsDecodeGatewayShapes() throws {
        let agents = try JSONDecoder().decode(
            GaryxAgentsPage.self,
            from: Data(
                """
                {
                  "agents": [
                    {
                      "agent_id": "codex",
                      "display_name": "Codex",
                      "provider_type": "codex_app_server",
                      "model": "provider default",
                      "default_workspace_dir": "/workspace/project",
                      "built_in": true,
                      "standalone": true
                    }
                  ]
                }
                """.utf8
            )
        )
        let automations = try JSONDecoder().decode(
            GaryxAutomationsPage.self,
            from: Data(
                """
                {
                  "automations": [
                    {
                      "id": "automation-test",
                      "label": "Daily Review",
                      "prompt": "Summarize open tasks.",
                      "agent_id": "codex",
                      "enabled": true,
                      "workspace_dir": "/workspace/project",
                      "target_thread_id": "thread::target",
                      "thread_id": "thread::automation",
                      "next_run": "2026-03-01T09:00:00Z",
                      "last_status": "success"
                    }
                  ]
                }
                """.utf8
            )
        )
        let skills = try JSONDecoder().decode(
            GaryxSkillsPage.self,
            from: Data(
                """
                {
                  "skills": [
                    {
                      "id": "mobile-skill",
                      "name": "Mobile Skill",
                      "description": "A synthetic skill.",
                      "installed": true,
                      "enabled": true,
                      "source_path": "/workspace/skills/mobile-skill"
                    }
                  ]
                }
                """.utf8
            )
        )

        XCTAssertEqual(agents.agents.first?.id, "codex")
        XCTAssertEqual(automations.automations.first?.workspacePath, "/workspace/project")
        XCTAssertEqual(automations.automations.first?.targetThreadId, "thread::target")
        XCTAssertEqual(skills.skills.first?.name, "Mobile Skill")
    }

    func testTaskSummaryMergesEnvelopeAndNestedTask() throws {
        let data = Data(
            """
            {
              "thread_id": "thread::task",
              "task_id": "task::1",
              "number": 1,
              "status": "in_progress",
              "runtime_agent_id": "codex",
              "task": {
                "number": 1,
                "title": "Ship mobile parity",
                "status": "in_progress",
                "assignee": { "kind": "agent", "agent_id": "codex" },
                "updated_at": "2026-03-01T09:00:00Z"
              }
            }
            """.utf8
        )

        let summary = try JSONDecoder().decode(GaryxTaskSummary.self, from: data)

        XCTAssertEqual(summary.id, "task::1")
        XCTAssertEqual(summary.threadId, "thread::task")
        XCTAssertEqual(summary.runtimeAgentId, "codex")
        XCTAssertEqual(summary.title, "Ship mobile parity")
        XCTAssertEqual(summary.assigneeLabel, "codex")
    }

    func testThreadTranscriptDecodesGatewayHistoryEnvelope() throws {
        let data = Data(
            """
            {
              "ok": true,
              "messages": [
                {
                  "index": 2,
                  "role": "assistant",
                  "kind": "assistant_reply",
                  "text": "done",
                  "content": "done",
                  "message": {
                    "role": "assistant",
                    "content": [
                      {
                        "type": "text",
                        "text": "inspect this"
                      },
                      {
                        "type": "image",
                        "name": "prompt-image.png",
                        "media_type": "image/png",
                        "source": {
                          "type": "base64",
                          "media_type": "image/png",
                          "data": "dGVzdA=="
                        }
                      }
                    ]
                  },
                  "timestamp": "2026-03-01T00:00:00Z",
                  "tool_related": false,
                  "likely_user_visible": true
                },
                {
                  "index": 3,
                  "role": "tool_result",
                  "kind": "tool_result",
                  "text": "ran test",
                  "tool_related": true,
                  "likely_user_visible": false
                }
              ],
              "pending_user_inputs": [
                {
                  "id": "pending-test",
                  "run_id": "run-test",
                  "text": "approve?",
                  "content": [
                    {
                      "type": "text",
                      "text": "approve?"
                    }
                  ],
                  "status": "awaiting_ack"
                }
              ],
              "thread_runtime": {
                "provider_type": "codex_app_server",
                "provider_label": "Codex",
                "sdk_session_id": "session-test",
                "active_run": {
                  "run_id": "run-test",
                  "provider_type": "codex_app_server",
                  "provider_label": "Codex",
                  "assistant_response": "streaming",
                  "updated_at": "2026-03-01T00:01:00Z",
                  "pending_user_input_count": 1
                }
              },
              "message_stats": {
                "returned_messages": 2,
                "returned_start_index": 2,
                "returned_end_index": 4,
                "has_more_before": true,
                "next_before_index": 2
              }
            }
            """.utf8
        )

        let transcript = try JSONDecoder().decode(GaryxThreadTranscript.self, from: data)

        XCTAssertTrue(transcript.ok)
        XCTAssertEqual(transcript.messages.map(\.id), ["history:2", "history:3"])
        XCTAssertEqual(transcript.messages[0].role, .assistant)
        XCTAssertEqual(transcript.messages[0].content, .string("done"))
        XCTAssertEqual(
            transcript.messages[0].message,
            .object([
                "role": .string("assistant"),
                "content": .array([
                    .object([
                        "type": .string("text"),
                        "text": .string("inspect this"),
                    ]),
                    .object([
                        "type": .string("image"),
                        "name": .string("prompt-image.png"),
                        "media_type": .string("image/png"),
                        "source": .object([
                            "type": .string("base64"),
                            "media_type": .string("image/png"),
                            "data": .string("dGVzdA=="),
                        ]),
                    ]),
                ]),
            ])
        )
        XCTAssertEqual(transcript.messages[1].role, .toolResult)
        XCTAssertTrue(transcript.messages[1].toolRelated)
        XCTAssertEqual(transcript.pendingUserInputs.first?.id, "pending-test")
        XCTAssertEqual(transcript.pendingUserInputs.first?.runId, "run-test")
        XCTAssertEqual(
            transcript.pendingUserInputs.first?.content,
            .array([
                .object([
                    "type": .string("text"),
                    "text": .string("approve?"),
                ]),
            ])
        )
        XCTAssertEqual(transcript.pendingUserInputs.first?.active, true)
        XCTAssertEqual(transcript.threadRuntime?.providerType, "codex_app_server")
        XCTAssertEqual(transcript.threadRuntime?.activeRun?.runId, "run-test")
        XCTAssertEqual(transcript.threadRuntime?.activeRun?.pendingUserInputCount, 1)
        XCTAssertEqual(transcript.pageInfo?.returnedMessages, 2)
        XCTAssertEqual(transcript.pageInfo?.returnedStartIndex, 2)
        XCTAssertEqual(transcript.pageInfo?.returnedEndIndex, 4)
        XCTAssertEqual(transcript.pageInfo?.hasMoreBefore, true)
        XCTAssertEqual(transcript.pageInfo?.nextBeforeIndex, 2)
    }

    func testTranscriptToolTraceClassifierIncludesAssistantToolRelatedMessages() throws {
        let transcript = try JSONDecoder().decode(
            GaryxThreadTranscript.self,
            from: Data(
                """
                {
                  "ok": true,
                  "messages": [
                    {
                      "index": 1,
                      "role": "assistant",
                      "kind": "tool_trace",
                      "tool_related": true,
                      "likely_user_visible": false,
                      "text": "Read App.swift",
                      "message": {
                        "tool_name": "Read",
                        "input": { "file_path": "App.swift" }
                      }
                    },
                    {
                      "index": 2,
                      "role": "assistant",
                      "kind": "tool_trace",
                      "tool_related": true,
                      "likely_user_visible": false,
                      "text": "Read complete",
                      "message": {
                        "tool_use_result": true,
                        "tool_use_id": "toolu-test",
                        "content": "Read complete"
                      }
                    },
                    {
                      "index": 3,
                      "role": "user",
                      "kind": "user_input",
                      "tool_related": true,
                      "likely_user_visible": true,
                      "text": "Tool-related user text"
                    },
                    {
                      "index": 4,
                      "role": "assistant",
                      "kind": "assistant_reply",
                      "tool_related": false,
                      "likely_user_visible": true,
                      "text": "Final answer"
                    }
                  ],
                  "pending_user_inputs": [],
                  "message_stats": { "returned_messages": 4 }
                }
                """.utf8
            )
        )

        XCTAssertEqual(
            GaryxMobileTranscriptToolTraceClassifier.kind(for: transcript.messages[0]),
            .toolUse
        )
        XCTAssertEqual(
            GaryxMobileTranscriptToolTraceClassifier.kind(for: transcript.messages[1]),
            .toolResult
        )
        XCTAssertNil(GaryxMobileTranscriptToolTraceClassifier.kind(for: transcript.messages[2]))
        XCTAssertNil(GaryxMobileTranscriptToolTraceClassifier.kind(for: transcript.messages[3]))
    }

    func testStructuredContentRendererExtractsTextAndAttachments() {
        let content: GaryxJSONValue = .array([
            .object([
                "type": .string("text"),
                "text": .string("Review this"),
            ]),
            .object([
                "type": .string("image"),
                "name": .string("screen.png"),
                "media_type": .string("image/png"),
                "source": .object([
                    "type": .string("base64"),
                    "media_type": .string("image/png"),
                    "data": .string("dGVzdA=="),
                ]),
            ]),
            .object([
                "type": .string("file"),
                "path": .string("/workspace/notes.txt"),
                "media_type": .string("text/plain"),
            ]),
        ])

        let attachments = GaryxStructuredContentRenderer.attachments(from: content)

        XCTAssertEqual(GaryxStructuredContentRenderer.text(from: content), "Review this")
        XCTAssertEqual(GaryxStructuredContentRenderer.summaryText(from: content), "Review this\n\n[1 image, 1 file]")
        XCTAssertEqual(attachments.count, 2)
        XCTAssertEqual(attachments[0].kind, "image")
        XCTAssertEqual(attachments[0].name, "screen.png")
        XCTAssertEqual(attachments[0].mediaType, "image/png")
        XCTAssertEqual(attachments[0].dataUrl, "data:image/png;base64,dGVzdA==")
        XCTAssertEqual(attachments[1].kind, "file")
        XCTAssertEqual(attachments[1].name, "notes.txt")
    }

    func testStructuredContentRendererUsesAttachmentAwareMergeKeys() {
        let image = GaryxContentAttachmentDescriptor(
            id: "image-test",
            kind: "image",
            name: "screen.png",
            mediaType: "image/png",
            dataUrl: "data:image/png;base64,dGVzdA=="
        )
        let file = GaryxContentAttachmentDescriptor(
            id: "file-test",
            kind: "file",
            name: "notes.txt",
            mediaType: "text/plain",
            path: "/workspace/notes.txt"
        )

        XCTAssertEqual(
            GaryxStructuredContentRenderer.userMergeKey(text: "", attachments: [image]),
            "[1 image]"
        )
        XCTAssertEqual(
            GaryxStructuredContentRenderer.userMergeKey(text: "[1 image]", attachments: [image]),
            "[1 image]"
        )
        XCTAssertEqual(
            GaryxStructuredContentRenderer.userMergeKey(text: "Continue", attachments: [image]),
            "Continue"
        )
        XCTAssertEqual(
            GaryxStructuredContentRenderer.userMergeKey(text: "", attachments: [image, file]),
            "[1 image, 1 file]"
        )
    }

    func testStructuredContentRendererHandlesOutOfRangeNumericAttachmentIds() {
        let content: GaryxJSONValue = .array([
            .object([
                "type": .string("image"),
                "id": .number(1e20),
                "media_type": .string("image/png"),
                "source": .object([
                    "type": .string("base64"),
                    "media_type": .string("image/png"),
                    "data": .string("dGVzdA=="),
                ]),
            ]),
        ])

        let attachments = GaryxStructuredContentRenderer.attachments(from: content)

        XCTAssertEqual(attachments.count, 1)
        XCTAssertTrue(attachments[0].id.hasPrefix("1e+20-"))
    }

    func testURLBuilderEncodesThreadHistoryQueryItems() throws {
        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/garyx/"))
            )
        )

        let url = try client.url(
            for: "/api/threads/history",
            queryItems: [
                URLQueryItem(name: "thread_id", value: "thread::a&b"),
                URLQueryItem(name: "include_tool_messages", value: "false"),
            ]
        )
        let components = try XCTUnwrap(URLComponents(url: url, resolvingAgainstBaseURL: false))

        XCTAssertEqual(url.path(), "/garyx/api/threads/history")
        XCTAssertEqual(
            components.queryItems?.first(where: { $0.name == "thread_id" })?.value,
            "thread::a&b"
        )
        XCTAssertEqual(
            components.queryItems?.first(where: { $0.name == "include_tool_messages" })?.value,
            "false"
        )
    }

    func testWorkspaceFilesRequestUsesGatewayCamelCaseQueryItems() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        GaryxURLProtocolStub.requestHandler = { request in
            XCTAssertEqual(request.httpMethod, "GET")
            XCTAssertEqual(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)?.percentEncodedPath,
                "/garyx/api/workspace-files"
            )
            let queryItems = URLComponents(
                url: try XCTUnwrap(request.url),
                resolvingAgainstBaseURL: false
            )?.queryItems ?? []
            XCTAssertEqual(queryItems.first(where: { $0.name == "workspaceDir" })?.value, "/workspace/project")
            XCTAssertEqual(queryItems.first(where: { $0.name == "workspace_dir" })?.value, nil)
            XCTAssertEqual(queryItems.first(where: { $0.name == "path" })?.value, "Sources")
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (
                response,
                Data(
                    """
                    {
                      "workspaceDir": "/workspace/project",
                      "directoryPath": "Sources",
                      "entries": []
                    }
                    """.utf8
                )
            )
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/garyx"))
            ),
            session: session
        )

        let listing = try await client.listWorkspaceFiles(
            workspaceDir: "/workspace/project",
            directoryPath: "Sources"
        )

        XCTAssertEqual(listing.workspaceDir, "/workspace/project")
        XCTAssertEqual(listing.directoryPath, "Sources")
    }

    func testGetThreadUsesMetadataEndpointAndEncodesThreadId() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        GaryxURLProtocolStub.requestHandler = { request in
            XCTAssertEqual(request.httpMethod, "GET")
            XCTAssertEqual(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)?.percentEncodedPath,
                "/garyx/api/threads/thread%3A%3Atest%2Fchild"
            )
            XCTAssertEqual(request.value(forHTTPHeaderField: "Accept"), "application/json")
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer test token")
            XCTAssertEqual(request.value(forHTTPHeaderField: "X-Garyx-Proxy"), "proxy-token")
            XCTAssertEqual(request.value(forHTTPHeaderField: "X-Trace-Id"), "trace-123")
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (
                response,
                Data(
                    """
                    {
                      "thread_id": "thread::test/child",
                      "label": "Pinned child thread"
                    }
                    """.utf8
                )
            )
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/garyx")),
                authToken: "test token",
                customHeaders: [
                    "X-Garyx-Proxy": "proxy-token",
                    "X-Trace-Id": "trace-123",
                ]
            ),
            session: session
        )

        let thread = try await client.getThread(threadId: "thread::test/child")

        XCTAssertEqual(thread.id, "thread::test/child")
        XCTAssertEqual(thread.title, "Pinned child thread")
    }

    func testThreadHistoryRequestEncodesPaginationQueryItems() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        GaryxURLProtocolStub.requestHandler = { request in
            XCTAssertEqual(request.httpMethod, "GET")
            XCTAssertEqual(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)?.percentEncodedPath,
                "/garyx/api/threads/history"
            )
            let queryItems = URLComponents(
                url: try XCTUnwrap(request.url),
                resolvingAgainstBaseURL: false
            )?.queryItems ?? []
            XCTAssertEqual(queryItems.first(where: { $0.name == "thread_id" })?.value, "thread::test/child")
            XCTAssertEqual(queryItems.first(where: { $0.name == "limit" })?.value, "42")
            XCTAssertEqual(queryItems.first(where: { $0.name == "before_index" })?.value, "120")
            XCTAssertEqual(queryItems.first(where: { $0.name == "user_query_limit" })?.value, "10")
            XCTAssertEqual(queryItems.first(where: { $0.name == "include_tool_messages" })?.value, "false")
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (
                response,
                Data(
                    """
                    {
                      "ok": true,
                      "messages": [],
                      "pending_user_inputs": [],
                      "message_stats": {
                        "returned_messages": 0,
                        "has_more_before": false,
                        "next_before_index": null
                      }
                    }
                    """.utf8
                )
            )
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/garyx")),
                authToken: "test token"
            ),
            session: session
        )

        let transcript = try await client.threadHistory(
            threadId: "thread::test/child",
            limit: 42,
            beforeIndex: 120,
            userQueryLimit: 10,
            includeToolMessages: false
        )

        XCTAssertTrue(transcript.ok)
        XCTAssertEqual(transcript.pageInfo?.returnedMessages, 0)
        XCTAssertEqual(transcript.pageInfo?.hasMoreBefore, false)
        XCTAssertNil(transcript.pageInfo?.nextBeforeIndex)
    }

    func testMacParityPayloadsDecodeGatewayShapes() throws {
        let automations = try JSONDecoder().decode(
            GaryxAutomationsPage.self,
            from: Data(
                """
                {
                  "automations": [
                    {
                      "id": "automation-test",
                      "label": "Interval Review",
                      "prompt": "Review open work.",
                      "agentId": "codex",
                      "enabled": true,
                      "workspaceDir": "/workspace/project",
                      "targetThreadId": "thread::target",
                      "nextRun": "2026-03-01T09:00:00Z",
                      "lastStatus": "success",
                      "schedule": { "kind": "interval", "hours": 6 }
                    }
                  ]
                }
                """.utf8
            )
        )
        let workspace = try JSONDecoder().decode(
            GaryxWorkspaceFileListing.self,
            from: Data(
                """
                {
                  "workspaceDir": "/workspace/project",
                  "directoryPath": "Sources",
                  "entries": [
                    {
                      "path": "Sources/App.swift",
                      "name": "App.swift",
                      "entryType": "file",
                      "size": 128,
                      "mediaType": "text/x-swift",
                      "hasChildren": false
                    }
                  ]
                }
                """.utf8
            )
        )
        let gitStatus = try JSONDecoder().decode(
            GaryxWorkspaceGitStatus.self,
            from: Data(
                """
                {
                  "workspace_dir": "/workspace/project",
                  "is_git_repo": true,
                  "repo_root": "/workspace/project",
                  "current_branch": "main",
                  "is_dirty": false
                }
                """.utf8
            )
        )
        let directoryListing = try JSONDecoder().decode(
            GaryxWorkspaceDirectoryListing.self,
            from: Data(
                """
                {
                  "path": "/workspace",
                  "parentPath": "/",
                  "entries": [
                    { "name": "project", "path": "/workspace/project" }
                  ]
                }
                """.utf8
            )
        )
        let commands = try JSONDecoder().decode(
            GaryxSlashCommandsPage.self,
            from: Data(
                """
                {
                  "commands": [
                    { "name": "ship", "description": "Ship a task.", "prompt": "Finish and verify." }
                  ]
                }
                """.utf8
            )
        )
        let mcp = try JSONDecoder().decode(
            GaryxMcpServersPage.self,
            from: Data(
                """
                {
                  "servers": [
                    {
                      "name": "test-server",
                      "transport": "stdio",
                      "command": "node",
                      "args": ["server.js"],
                      "env": { "TOKEN": "${TOKEN}" },
                      "enabled": true,
                      "working_dir": "/workspace/project"
                    }
                  ]
                }
                """.utf8
            )
        )
        let bots = try JSONDecoder().decode(
            GaryxBotConsolesPage.self,
            from: Data(
                """
                {
                  "bots": [
                    {
                      "id": "telegram::main",
                      "channel": "telegram",
                      "account_id": "main",
                      "title": "Test Bot",
                      "subtitle": "telegram / main",
                      "status": "connected",
                      "endpoint_count": 2,
                      "bound_endpoint_count": 1,
                      "workspace_dir": "/workspace/project",
                      "main_endpoint_thread_id": "thread::main",
                      "default_open_thread_id": "thread::test"
                    }
                  ]
                }
                """.utf8
            )
        )

        XCTAssertEqual(automations.automations.first?.schedule.hours, 6)
        XCTAssertEqual(automations.automations.first?.targetThreadId, "thread::target")
        XCTAssertEqual(workspace.entries.first?.path, "Sources/App.swift")
        XCTAssertTrue(gitStatus.canUseWorktree)
        XCTAssertEqual(gitStatus.currentBranch, "main")
        XCTAssertEqual(directoryListing.parentPath, "/")
        XCTAssertEqual(directoryListing.entries.first?.path, "/workspace/project")
        XCTAssertEqual(commands.commands.first?.prompt, "Finish and verify.")
        XCTAssertEqual(mcp.servers.first?.args, ["server.js"])
        XCTAssertEqual(bots.bots.first?.boundEndpointCount, 1)
        XCTAssertEqual(bots.bots.first?.mainThreadId, "thread::main")
    }

    func testAutomationTypedAgentAndValidationWireFieldsDecodeWithoutClaudeFallback() throws {
        let page = try JSONDecoder().decode(
            GaryxAutomationsPage.self,
            from: Data(
                """
                {
                  "automations": [
                    {
                      "id": "cron::target",
                      "label": "Target job",
                      "prompt": "Continue",
                      "agentId": null,
                      "agentResolution": "follow_thread",
                      "effectiveAgentId": "codex",
                      "enabled": true,
                      "workspaceDir": "",
                      "targetThreadId": "thread::target",
                      "threadMode": "target",
                      "nextRun": "2026-07-17T00:00:00Z",
                      "lastStatus": "never_run",
                      "schedule": { "kind": "interval", "hours": 6 },
                      "validationState": "invalid",
                      "validationError": "target has no canonical binding"
                    }
                  ]
                }
                """.utf8
            )
        )

        let automation = try XCTUnwrap(page.automations.first)
        XCTAssertNil(automation.agentId)
        XCTAssertEqual(automation.agentResolution, .followThread)
        XCTAssertEqual(automation.effectiveAgentId, "codex")
        XCTAssertEqual(automation.validationState, .invalid)
        XCTAssertEqual(automation.validationError, "target has no canonical binding")
    }

    func testMacParityRequestsEncodeGatewayShapes() throws {
        let automation = GaryxAutomationCreateRequest(
            label: "Interval Review",
            prompt: "Review open work.",
            agentId: "codex",
            workspaceDir: "/workspace/project",
            targetThreadId: "thread::target",
            schedule: .interval(hours: 6),
            enabled: true
        )
        let automationObject = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(automation)
        ) as? [String: Any]
        let automationSchedule = automationObject?["schedule"] as? [String: Any]

        XCTAssertEqual(automationObject?["agentId"] as? String, "codex")
        XCTAssertEqual(automationObject?["workspaceDir"] as? String, "/workspace/project")
        XCTAssertEqual(automationObject?["targetThreadId"] as? String, "thread::target")
        XCTAssertEqual(automationSchedule?["kind"] as? String, "interval")
        XCTAssertEqual(automationSchedule?["hours"] as? Int, 6)

        let unchangedAutomationUpdate = GaryxAutomationUpdateRequest(label: "Interval Review")
        let unchangedAutomationUpdateObject = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(unchangedAutomationUpdate)
        ) as? [String: Any]
        XCTAssertFalse(unchangedAutomationUpdateObject?.keys.contains("targetThreadId") ?? true)
        XCTAssertFalse(unchangedAutomationUpdateObject?.keys.contains("agentId") ?? true)

        let changedAutomationAgent = GaryxAutomationUpdateRequest(agentId: "codex")
        let changedAutomationAgentObject = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(changedAutomationAgent)
        ) as? [String: Any]
        XCTAssertEqual(changedAutomationAgentObject?["agentId"] as? String, "codex")

        let boundAutomationUpdate = GaryxAutomationUpdateRequest(targetThreadId: "thread::target")
        let boundAutomationUpdateObject = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(boundAutomationUpdate)
        ) as? [String: Any]
        XCTAssertEqual(boundAutomationUpdateObject?["targetThreadId"] as? String, "thread::target")

        let clearedAutomationUpdate = GaryxAutomationUpdateRequest(
            workspaceDir: "/workspace/project",
            clearsTargetThreadId: true
        )
        let clearedAutomationUpdateObject = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(clearedAutomationUpdate)
        ) as? [String: Any]
        XCTAssertTrue(clearedAutomationUpdateObject?["targetThreadId"] is NSNull)
        XCTAssertEqual(clearedAutomationUpdateObject?["workspaceDir"] as? String, "/workspace/project")

        let mcp = GaryxMcpServerRequest(
            name: "test-server",
            transport: "stdio",
            command: "node",
            args: ["server.js"],
            env: ["TOKEN": "${TOKEN}"],
            enabled: true,
            workingDir: "/workspace/project"
        )
        let mcpObject = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(mcp)
        ) as? [String: Any]

        XCTAssertEqual(mcpObject?["working_dir"] as? String, "/workspace/project")
        XCTAssertEqual(mcpObject?["args"] as? [String], ["server.js"])

        let thread = GaryxCreateThreadRequest(
            workspaceDir: "/workspace/project",
            workspaceMode: "local",
            agentId: "codex",
            metadata: ["client": "garyx-mobile"]
        )
        let threadObject = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(thread)
        ) as? [String: Any]

        XCTAssertNil(threadObject?["label"])
        XCTAssertEqual(threadObject?["workspaceDir"] as? String, "/workspace/project")
        XCTAssertEqual(threadObject?["agentId"] as? String, "codex")
    }

    func testAutomationScheduleEncodesAndDecodesCalendarShapes() throws {
        let monthly = GaryxAutomationSchedule.monthly(day: 31, time: "08:45", timezone: "Asia/Shanghai")
        let monthlyObject = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(monthly)
        ) as? [String: Any]
        XCTAssertEqual(monthlyObject?["kind"] as? String, "monthly")
        XCTAssertEqual(monthlyObject?["day"] as? Int, 31)
        XCTAssertEqual(monthlyObject?["time"] as? String, "08:45")
        XCTAssertEqual(monthlyObject?["timezone"] as? String, "Asia/Shanghai")

        let decodedDaily = try JSONDecoder().decode(
            GaryxAutomationSchedule.self,
            from: Data(
                """
                {
                  "kind": "daily",
                  "time": "09:30",
                  "weekdays": ["mo", "tu"],
                  "timezone": "UTC"
                }
                """.utf8
            )
        )
        XCTAssertEqual(decodedDaily.kind, .daily)
        XCTAssertEqual(decodedDaily.time, "09:30")
        XCTAssertEqual(decodedDaily.weekdays, ["mo", "tu"])
        XCTAssertEqual(decodedDaily.timezone, "UTC")

        let once = GaryxAutomationSchedule.once(at: "2026-03-01T09:00")
        let onceObject = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(once)
        ) as? [String: Any]
        XCTAssertEqual(onceObject?["kind"] as? String, "once")
        XCTAssertEqual(onceObject?["at"] as? String, "2026-03-01T09:00")
    }

    func testAutomationThreadsPageDecodesGatewayShape() throws {
        let page = try JSONDecoder().decode(
            GaryxAutomationThreadsPage.self,
            from: Data(
                """
                {
                  "automationId": "automation::daily",
                  "automationLabel": "Daily Review",
                  "automationDeleted": false,
                  "items": [
                    {
                      "automationId": "automation::daily",
                      "runId": "run-1",
                      "threadId": "thread::generated",
                      "workspaceDir": "/Users/test/project",
                      "agentId": "codex",
                      "automationLabel": "Daily Review",
                      "automationDeleted": false,
                      "status": "running",
                      "startedAt": "2026-05-28T00:00:00Z",
                      "thread": {
                        "threadId": "thread::generated",
                        "title": "Daily Review",
                        "workspaceDir": "/Users/test/project",
                        "agentId": "codex",
                        "automationId": "automation::daily",
                        "automationThreadMode": "generated_thread",
                        "excludeFromRecent": true,
                        "messageCount": 2
                      }
                    }
                  ],
                  "count": 1,
                  "total": 1,
                  "limit": 50,
                  "offset": 0,
                  "hasMore": false
                }
                """.utf8
            )
        )

        XCTAssertEqual(page.automationId, "automation::daily")
        XCTAssertEqual(page.items.first?.thread?.id, "thread::generated")
        XCTAssertEqual(page.items.first?.thread?.automationThreadMode, "generated_thread")
        XCTAssertEqual(page.items.first?.thread?.excludeFromRecent, true)
    }

    func testMacParityAgentAndChannelPayloadsDecodeGatewayShapes() throws {
        let agents = try JSONDecoder().decode(
            GaryxAgentsPage.self,
            from: Data(
                """
                {
                  "default_agent_id": "agent-disabled",
                  "effective_default_agent_id": "agent-test",
                  "agents": [
                    {
                      "agent_id": "agent-test",
                      "display_name": "Test Agent",
                      "provider_type": "codex_app_server",
                      "model": "gpt-test",
                      "model_reasoning_effort": "medium",
                      "model_service_tier": "default",
                      "provider_env": { "TOKEN": "${TOKEN}" },
                      "default_workspace_dir": "/workspace/project",
                      "avatar_data_url": "data:image/png;base64,dGVzdA==",
                      "system_prompt": "Help with test work.",
                      "built_in": false,
                      "standalone": true,
                      "enabled": true,
                      "created_at": "2026-03-01T09:00:00Z",
                      "updated_at": "2026-03-01T09:10:00Z"
                    },
                    {
                      "agentId": "agent-remote-avatar",
                      "displayName": "Remote Avatar Agent",
                      "providerType": "codex_app_server",
                      "enabled": false,
                      "avatarURL": "https://example.test/avatar.png"
                    }
                  ]
                }
                """.utf8
            )
        )
        let plugins = try JSONDecoder().decode(
            GaryxChannelPluginCatalogPage.self,
            from: Data(
                """
                {
                  "plugins": [
                    {
                      "id": "test-channel",
                      "display_name": "Test Channel",
                      "description": "Synthetic channel plugin.",
                      "icon_data_url": "data:image/png;base64,dGVzdA==",
                      "schema": { "token": { "type": "string" } },
                      "config_methods": [{ "kind": "auth_flow", "title": "Login" }]
                    }
                  ]
                }
                """.utf8
            )
        )
        let configuredBots = try JSONDecoder().decode(
            GaryxConfiguredBotsPage.self,
            from: Data(
                """
                {
                  "bots": [
                    {
                      "channel": "api",
                      "account_id": "account-test",
                      "display_name": "Test Bot",
                      "enabled": true,
                      "agent_id": "agent-test",
                      "effective_agent_id": "agent-test",
                      "workspace_dir": "/workspace/project",
                      "root_behavior": "open_default",
                      "main_endpoint_status": "resolved",
                      "default_open_thread_id": "thread::test"
                    }
                  ]
                }
                """.utf8
            )
        )
        let botConsoles = try JSONDecoder().decode(
            GaryxBotConsolesPage.self,
            from: Data(
                """
                {
                  "bots": [
                    {
                      "id": "api::account-test",
                      "channel": "api",
                      "account_id": "account-test",
                      "title": "Test Bot",
                      "subtitle": "API / account-test",
                      "agent_id": "agent-test",
                      "effective_agent_id": "agent-test",
                      "root_behavior": "open_default",
                      "status": "connected",
                      "latest_activity": "2026-03-01T09:15:00Z",
                      "endpoint_count": 1,
                      "bound_endpoint_count": 1,
                      "workspace_dir": "/workspace/project",
                      "main_endpoint_thread_id": "thread::main",
                      "default_open_thread_id": "thread::test",
                      "conversation_nodes": [
                        {
                          "id": "conversation-test",
                          "endpoint": {
                            "endpoint_key": "api::account-test::1000000001",
                            "channel": "api",
                            "account_id": "account-test",
                            "display_label": "Test Conversation",
                            "thread_id": "thread::test",
                            "thread_label": "Test Thread",
                            "conversation_kind": "private",
                            "conversation_label": "Test Conversation"
                          },
                          "kind": "private",
                          "title": "Test Conversation",
                          "badge": null,
                          "latest_activity": "2026-03-01T09:15:00Z",
                          "openable": true
                        }
                      ]
                    }
                  ]
                }
                """.utf8
            )
        )
        let binding = try JSONDecoder().decode(
            GaryxBotBindingResult.self,
            from: Data(
                """
                {
                  "ok": true,
                  "bot_id": "telegram:main",
                  "channel": "telegram",
                  "account_id": "main",
                  "main_endpoint_status": "resolved",
                  "current_thread_status": "bound",
                  "current_thread_id": "thread::test",
                  "action": "bind",
                  "thread_id": "thread::test",
                  "previous_thread_id": null,
                  "endpoint_key": "telegram::main::1000000001"
                }
                """.utf8
            )
        )

        XCTAssertEqual(agents.agents.first?.providerEnv["TOKEN"], "${TOKEN}")
        XCTAssertEqual(agents.defaultAgentId, "agent-disabled")
        XCTAssertEqual(agents.effectiveDefaultAgentId, "agent-test")
        XCTAssertEqual(agents.agents.map(\.enabled), [true, false])
        XCTAssertEqual(agents.agents.first?.systemPrompt, "Help with test work.")
        XCTAssertEqual(agents.agents.last?.avatarDataUrl, "https://example.test/avatar.png")
        XCTAssertEqual(plugins.plugins.first?.iconDataUrl, "data:image/png;base64,dGVzdA==")
        XCTAssertEqual(plugins.plugins.first?.configMethods.first?.kind, "auth_flow")
        XCTAssertEqual(configuredBots.bots.first?.agentId, "agent-test")
        XCTAssertEqual(configuredBots.bots.first?.effectiveAgentId, "agent-test")
        XCTAssertEqual(configuredBots.bots.first?.mainThreadId, nil)
        XCTAssertEqual(botConsoles.bots.first?.agentId, "agent-test")
        XCTAssertEqual(botConsoles.bots.first?.effectiveAgentId, "agent-test")
        XCTAssertEqual(botConsoles.bots.first?.mainThreadId, "thread::main")
        XCTAssertEqual(botConsoles.bots.first?.conversationNodes.first?.endpoint.threadId, "thread::test")
        XCTAssertEqual(binding.endpointKey, "telegram::main::1000000001")
    }

    func testMacParityAttachmentBotAndLogPayloadsRoundTripGatewayShapes() throws {
        let upload = try JSONDecoder().decode(
            GaryxUploadChatAttachmentsResult.self,
            from: Data(
                """
                {
                  "files": [
                    {
                      "kind": "image",
                      "path": "/workspace/tmp/prompt-image.png",
                      "name": "prompt-image.png",
                      "media_type": "image/png"
                    }
                  ]
                }
                """.utf8
            )
        )
        let logs = try JSONDecoder().decode(
            GaryxThreadLogChunk.self,
            from: Data(
                """
                {
                  "thread_id": "thread::test",
                  "path": "/workspace/project/.garyx/logs/thread.log",
                  "text": "accepted\\ndone\\n",
                  "cursor": 14,
                  "reset": true
                }
                """.utf8
            )
        )
        XCTAssertEqual(upload.files.first?.mediaType, "image/png")
        XCTAssertEqual(logs.cursor, 14)

        let botRequest = GaryxBotBindingRequest(botId: "telegram:main", threadId: "thread::test")
        let botObject = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(botRequest)
        ) as? [String: Any]

        XCTAssertEqual(botObject?["botId"] as? String, "telegram:main")
        XCTAssertEqual(botObject?["threadId"] as? String, "thread::test")
    }

    func testMobileThreadLinkRoundTripsWidgetURL() throws {
        let url = try XCTUnwrap(GaryxMobileThreadLink.make(threadId: " thread::recent "))

        XCTAssertEqual(url.absoluteString, "garyx://mobile/thread?threadId=thread::recent")
        XCTAssertEqual(GaryxMobileThreadLink.parse(url), "thread::recent")
        XCTAssertEqual(GaryxMobileThreadLink.parse(URL(string: "garyx://thread?id=thread::other")!), "thread::other")
        XCTAssertNil(GaryxMobileThreadLink.parse(URL(string: "garyx://mobile/connect?gatewayUrl=http://gateway.local")!))
    }

    func testWidgetStorePersistsScrollableRecentThreads() {
        let defaults = UserDefaults(suiteName: "garyx.mobile.widget.tests")!
        GaryxMobileWidgetStore.clear(defaults: defaults)
        let threads = (1...7).map { index in
            GaryxMobileWidgetThread(
                id: "thread::\(index)",
                title: "Thread \(index)",
                workspaceName: "Workspace",
                agentId: "agent::test",
                agentName: "Test Agent",
                avatarDataUrl: "data:image/png;base64,dGVzdA==",
                providerType: "codex",
                builtIn: true
            )
        }

        GaryxMobileWidgetStore.saveRecentThreads(
            threads,
            refreshedAt: Date(timeIntervalSince1970: 100),
            defaults: defaults
        )

        let snapshot = GaryxMobileWidgetStore.loadRecentThreads(defaults: defaults)
        XCTAssertEqual(snapshot.threads.map(\.id), [
            "thread::1",
            "thread::2",
            "thread::3",
            "thread::4",
            "thread::5",
            "thread::6",
            "thread::7",
        ])
        XCTAssertEqual(GaryxMobileWidgetStore.visibleThreadLimit, 5)
        XCTAssertEqual(snapshot.threads.first?.agentName, "Test Agent")
        XCTAssertEqual(snapshot.threads.first?.builtIn, true)
        XCTAssertEqual(snapshot.refreshedAt, Date(timeIntervalSince1970: 100))
        GaryxMobileWidgetStore.clear(defaults: defaults)
    }

    func testWidgetStoreCapsStoredThreadsForSnapshotSize() {
        let defaults = UserDefaults(suiteName: "garyx.mobile.widget.tests")!
        GaryxMobileWidgetStore.clear(defaults: defaults)
        let threads = (1...25).map { index in
            GaryxMobileWidgetThread(
                id: "thread::\(index)",
                title: "Thread \(index)",
                workspaceName: "Workspace"
            )
        }

        GaryxMobileWidgetStore.saveRecentThreads(threads, defaults: defaults)

        let snapshot = GaryxMobileWidgetStore.loadRecentThreads(defaults: defaults)
        XCTAssertEqual(snapshot.threads.count, GaryxMobileWidgetStore.storedThreadLimit)
        XCTAssertEqual(snapshot.threads.last?.id, "thread::20")
        GaryxMobileWidgetStore.clear(defaults: defaults)
    }

    // MARK: - Retry behavior

    func testRetryPolicyDelayUsesExponentialBackoffWithoutJitter() {
        let policy = GaryxGatewayRetryPolicy(
            maxAttempts: 4,
            initialDelay: 0.5,
            maxDelay: 4.0,
            backoffMultiplier: 2.0,
            jitter: 0
        )
        XCTAssertEqual(policy.delay(forAttempt: 1), 0.5, accuracy: 0.001)
        XCTAssertEqual(policy.delay(forAttempt: 2), 1.0, accuracy: 0.001)
        XCTAssertEqual(policy.delay(forAttempt: 3), 2.0, accuracy: 0.001)
        XCTAssertEqual(policy.delay(forAttempt: 4), 4.0, accuracy: 0.001)
        // Capped by maxDelay.
        XCTAssertEqual(policy.delay(forAttempt: 5), 4.0, accuracy: 0.001)
    }

    func testGatewayClientRetriesTransientServerErrorsOnIdempotentGet() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let attemptCount = GaryxAtomicCounter()
        GaryxURLProtocolStub.requestHandler = { request in
            let attempt = attemptCount.increment()
            let statusCode = attempt < 3 ? 503 : 200
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: statusCode,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            let body: String
            if statusCode == 200 {
                body = #"{"status":"ok"}"#
            } else {
                body = #"{"error":"upstream unavailable"}"#
            }
            return (response, Data(body.utf8))
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: GaryxGatewayRetryPolicy(
                maxAttempts: 3,
                initialDelay: 0.01,
                maxDelay: 0.01,
                backoffMultiplier: 1.0,
                jitter: 0
            )
        )

        let status = try await client.status()
        XCTAssertEqual(status.status, "ok")
        XCTAssertEqual(attemptCount.value(), 3)
    }

    func testGatewayClientDoesNotRetryNetworkConnectionLostOnMutation() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let attemptCount = GaryxAtomicCounter()
        GaryxURLProtocolStub.requestHandler = { request in
            let attempt = attemptCount.increment()
            if attempt == 1 {
                throw URLError(.networkConnectionLost)
            }
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            let body = """
            {
              "status": "queued",
              "thread_status": "queued",
              "client_intent_id": "intent-1",
              "pending_input_id": "pending-1",
              "thread_id": "thread::test"
            }
            """
            return (response, Data(body.utf8))
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: GaryxGatewayRetryPolicy(
                maxAttempts: 3,
                initialDelay: 0.01,
                maxDelay: 0.01,
                backoffMultiplier: 1.0,
                jitter: 0
            )
        )

        do {
            _ = try await client.streamInput(
                GaryxStreamInputRequest(
                    threadId: "thread::test",
                    clientIntentId: "intent-1",
                    message: "hello",
                    attachments: []
                )
            )
            XCTFail("Expected the first post-dispatch transport error to surface")
        } catch {
            XCTAssertEqual((error as? URLError)?.code, .networkConnectionLost)
            XCTAssertEqual(attemptCount.value(), 1)
        }
    }

    func testGatewayClientDoesNotRetry503OnNonIdempotentPost() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let attemptCount = GaryxAtomicCounter()
        GaryxURLProtocolStub.requestHandler = { request in
            _ = attemptCount.increment()
            // 503 could mean the request was partially processed; non-idempotent POSTs
            // (stream-input has no server dedup) must surface the error to the caller
            // so the user can explicitly retry.
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 503,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (response, Data(#"{"error":"service unavailable"}"#.utf8))
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: GaryxGatewayRetryPolicy(
                maxAttempts: 3,
                initialDelay: 0.01,
                maxDelay: 0.01,
                backoffMultiplier: 1.0,
                jitter: 0
            )
        )

        do {
            _ = try await client.streamInput(
                GaryxStreamInputRequest(
                    threadId: "thread::test",
                    clientIntentId: "intent-1",
                    message: "hello",
                    attachments: []
                )
            )
            XCTFail("Expected stream-input to surface 503 without retry")
        } catch let error as GaryxGatewayError {
            guard case .httpStatus(let code, _, _) = error else {
                XCTFail("Expected httpStatus error, got \(error)")
                return
            }
            XCTAssertEqual(code, 503)
            XCTAssertEqual(attemptCount.value(), 1)
        }
    }

    func testGatewayClientDoesNotRetry502OnMutation() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let attemptCount = GaryxAtomicCounter()
        GaryxURLProtocolStub.requestHandler = { request in
            _ = attemptCount.increment()
            let statusCode = 502
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: statusCode,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            let body = #"{"error":"bad gateway"}"#
            return (response, Data(body.utf8))
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: GaryxGatewayRetryPolicy(
                maxAttempts: 3,
                initialDelay: 0.01,
                maxDelay: 0.01,
                backoffMultiplier: 1.0,
                jitter: 0
            )
        )

        do {
            _ = try await client.streamInput(
                GaryxStreamInputRequest(
                    threadId: "thread::test",
                    clientIntentId: "intent-1",
                    message: "hello",
                    attachments: []
                )
            )
            XCTFail("Expected 502 to surface after one mutation attempt")
        } catch let error as GaryxGatewayError {
            guard case .httpStatus(let status, _, _) = error else {
                return XCTFail("Expected HTTP status, got \(error)")
            }
            XCTAssertEqual(status, 502)
            XCTAssertEqual(attemptCount.value(), 1)
        }
    }

    func testRetryClassifierIdentifiesConnectionEstablishmentErrors() {
        XCTAssertTrue(
            GaryxGatewayRetryClassifier.isConnectionEstablishmentError(URLError(.cannotConnectToHost))
        )
        XCTAssertTrue(
            GaryxGatewayRetryClassifier.isConnectionEstablishmentError(URLError(.networkConnectionLost))
        )
        XCTAssertTrue(
            GaryxGatewayRetryClassifier.isConnectionEstablishmentError(URLError(.notConnectedToInternet))
        )
        XCTAssertFalse(
            GaryxGatewayRetryClassifier.isConnectionEstablishmentError(URLError(.timedOut))
        )
        XCTAssertFalse(
            GaryxGatewayRetryClassifier.isConnectionEstablishmentError(URLError(.userCancelledAuthentication))
        )
    }

    func testRetryClassifierMatchesGatewayErrorStatuses() {
        for status in [408, 425, 429, 502, 503, 504] {
            XCTAssertTrue(
                GaryxGatewayRetryClassifier.isRetryableStatus(
                    status,
                    semantics: .readRetryable
                )
            )
            XCTAssertFalse(
                GaryxGatewayRetryClassifier.isRetryableStatus(
                    status,
                    semantics: .mutationSingleAttempt
                )
            )
        }
        XCTAssertFalse(
            GaryxGatewayRetryClassifier.isRetryableStatus(
                400,
                semantics: .readRetryable
            )
        )
        XCTAssertFalse(
            GaryxGatewayRetryClassifier.isRetryableStatus(
                404,
                semantics: .readRetryable
            )
        )
    }

    func testGatewayTransportHelpersHaveNoDefaultSemanticMode() throws {
        let packageRoot = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let source = try String(
            contentsOf: packageRoot
                .appendingPathComponent("Sources")
                .appendingPathComponent("GaryxMobileCore")
                .appendingPathComponent("GaryxGatewayClient.swift"),
            encoding: .utf8
        )
        XCTAssertTrue(source.contains("semantics: GaryxGatewayRequestSemantics,"))
        XCTAssertFalse(source.contains("semantics: GaryxGatewayRequestSemantics ="))
        XCTAssertFalse(source.contains("idempotent: Bool"))
        XCTAssertTrue(source.contains("semantics: .readRetryable"))
        XCTAssertTrue(source.contains("semantics: .mutationSingleAttempt"))
    }

    func testCapsuleHTMLFetchUsesAuthenticatedServeRouteAndReturnsUTF8Text() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        GaryxURLProtocolStub.requestHandler = { request in
            XCTAssertEqual(request.httpMethod, "GET")
            XCTAssertEqual(
                URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)?.percentEncodedPath,
                "/garyx/api/capsules/01900000-0000-7000-8000-000000000001/serve"
            )
            XCTAssertEqual(request.value(forHTTPHeaderField: "Accept"), "text/html")
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer test token")
            XCTAssertEqual(request.value(forHTTPHeaderField: "X-Garyx-Proxy"), "proxy-token")
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "text/html; charset=utf-8"]
                )
            )
            return (response, Data("<html><body>Capsule ✓</body></html>".utf8))
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/garyx")),
                authToken: "test token",
                customHeaders: ["X-Garyx-Proxy": "proxy-token"]
            ),
            session: session,
            retryPolicy: .disabled
        )

        let html = try await client.capsuleHTML(id: "01900000-0000-7000-8000-000000000001")

        XCTAssertEqual(html, "<html><body>Capsule ✓</body></html>")
    }

    func testListAndDeleteCapsulesUseGatewayRoutes() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        let counter = GaryxAtomicCounter()
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        GaryxURLProtocolStub.requestHandler = { request in
            let call = counter.increment()
            let path = URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)?.percentEncodedPath
            if call == 1 {
                XCTAssertEqual(request.httpMethod, "GET")
                XCTAssertEqual(path, "/garyx/api/capsules")
                let response = try XCTUnwrap(
                    HTTPURLResponse(
                        url: try XCTUnwrap(request.url),
                        statusCode: 200,
                        httpVersion: nil,
                        headerFields: ["Content-Type": "application/json"]
                    )
                )
                return (
                    response,
                    Data(
                        """
                        {
                          "capsules": [
                            {
                              "id": "01900000-0000-7000-8000-000000000001",
                              "title": "Synthetic Capsule",
                              "html_sha256": "abc123",
                              "byte_size": 42,
                              "revision": 1
                            }
                          ]
                        }
                        """.utf8
                    )
                )
            }
            XCTAssertEqual(request.httpMethod, "DELETE")
            XCTAssertEqual(path, "/garyx/api/capsules/01900000-0000-7000-8000-000000000001")
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (response, Data(#"{"deleted":true}"#.utf8))
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/garyx"))
            ),
            session: session,
            retryPolicy: .disabled
        )

        let capsules = try await client.listCapsules()
        XCTAssertEqual(capsules.map(\.id), ["01900000-0000-7000-8000-000000000001"])
        let result = try await client.deleteCapsule(id: "01900000-0000-7000-8000-000000000001")
        XCTAssertEqual(result.deleted, true)
        XCTAssertEqual(counter.value(), 2)
    }

    func testSetCapsuleFavoriteUsesPutAndDeleteRoutes() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        let counter = GaryxAtomicCounter()
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        GaryxURLProtocolStub.requestHandler = { request in
            let call = counter.increment()
            let path = URLComponents(
                url: try XCTUnwrap(request.url),
                resolvingAgainstBaseURL: false
            )?.percentEncodedPath
            XCTAssertEqual(
                path,
                "/garyx/api/capsules/01900000-0000-7000-8000-000000000001/favorite"
            )
            XCTAssertEqual(request.httpMethod, call == 1 ? "PUT" : "DELETE")
            let favorited = call == 1
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (
                response,
                Data(
                    """
                    {
                      "favorited": \(favorited),
                      "capsule": {
                        "id": "01900000-0000-7000-8000-000000000001",
                        "title": "Synthetic Capsule",
                        "favorited_at": \(favorited ? "\"2026-07-14T01:00:00Z\"" : "null")
                      }
                    }
                    """.utf8
                )
            )
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/garyx"))
            ),
            session: session,
            retryPolicy: .disabled
        )
        let id = "01900000-0000-7000-8000-000000000001"
        let favorited = try await client.setCapsuleFavorite(id: id, favorited: true)
        let unfavorited = try await client.setCapsuleFavorite(id: id, favorited: false)
        XCTAssertTrue(favorited.favorited)
        XCTAssertEqual(favorited.capsule.favoritedAt, "2026-07-14T01:00:00Z")
        XCTAssertFalse(unfavorited.favorited)
        XCTAssertNil(unfavorited.capsule.favoritedAt)
        XCTAssertEqual(counter.value(), 2)
    }

    func testMobileFileLinkParsesAbsoluteAndFileURLs() throws {
        XCTAssertEqual(
            GaryxMobileFileLink.localFilePath(from: "/workspace/project/docs/page.html?tab=preview#top"),
            "/workspace/project/docs/page.html"
        )
        XCTAssertEqual(
            GaryxMobileFileLink.localFilePath(from: "file:///workspace/project/docs/My%20File.md"),
            "/workspace/project/docs/My File.md"
        )
        XCTAssertEqual(
            GaryxMobileFileLink.localFilePath(from: "file:/workspace/project/assets/chart.png"),
            "/workspace/project/assets/chart.png"
        )
        XCTAssertNil(GaryxMobileFileLink.localFilePath(from: "https://example.test/docs/page.html"))
        XCTAssertNil(GaryxMobileFileLink.localFilePath(from: "docs/page.html"))
    }

    func testMobileFileLinkStripsTrailingLineAndColumnSuffix() {
        XCTAssertEqual(
            GaryxMobileFileLink.localFilePath(from: "/workspace/project/docs/plan.md:1"),
            "/workspace/project/docs/plan.md"
        )
        XCTAssertEqual(
            GaryxMobileFileLink.localFilePath(from: "/workspace/project/docs/plan.md:42:7"),
            "/workspace/project/docs/plan.md"
        )
        XCTAssertEqual(
            GaryxMobileFileLink.localFilePath(from: "file:///workspace/project/docs/plan.md:12"),
            "/workspace/project/docs/plan.md"
        )

        let absoluteTarget = GaryxMobileFileLink.previewTarget(
            fromLink: "/workspace/project/docs/plan.md:1",
            workspacePaths: ["/workspace/project"],
            currentWorkspaceDir: "/workspace/project",
            currentFilePath: nil
        )
        XCTAssertEqual(
            absoluteTarget,
            GaryxMobileWorkspaceFileTarget(
                workspaceDir: "/workspace/project",
                path: "docs/plan.md"
            )
        )

        let relativeTarget = GaryxMobileFileLink.previewTarget(
            fromLink: "docs/plan.md:1",
            workspacePaths: ["/workspace/project"],
            currentWorkspaceDir: "/workspace/project",
            currentFilePath: nil
        )
        XCTAssertEqual(
            relativeTarget,
            GaryxMobileWorkspaceFileTarget(
                workspaceDir: "/workspace/project",
                path: "docs/plan.md"
            )
        )
    }

    func testMobileFileLinkResolvesWorkspacePreviewTarget() {
        let target = GaryxMobileFileLink.previewTarget(
            forLocalFilePath: "/workspace/project/docs/page.html",
            workspacePaths: ["/workspace", "/workspace/project"]
        )

        XCTAssertEqual(
            target,
            GaryxMobileWorkspaceFileTarget(
                workspaceDir: "/workspace/project",
                path: "docs/page.html"
            )
        )
    }

    func testMobileFileLinkFallsBackToParentDirectoryWorkspace() {
        let target = GaryxMobileFileLink.previewTarget(
            forLocalFilePath: "/workspace/standalone/report.html",
            workspacePaths: []
        )

        XCTAssertEqual(
            target,
            GaryxMobileWorkspaceFileTarget(
                workspaceDir: "/workspace/standalone",
                path: "report.html"
            )
        )
    }

    func testMobileFileLinkResolvesRelativeLinksFromWorkspacePreview() {
        let target = GaryxMobileFileLink.previewTarget(
            fromLink: "../public/index.html#main",
            workspacePaths: ["/workspace/project"],
            currentWorkspaceDir: "/workspace/project",
            currentFilePath: "docs/notes/readme.md"
        )

        XCTAssertEqual(
            target,
            GaryxMobileWorkspaceFileTarget(
                workspaceDir: "/workspace/project",
                path: "docs/public/index.html"
            )
        )
    }

    func testMobileFileLinkRejectsRelativeLinksEscapingWorkspaceRoot() {
        // From docs/readme.md, ../../secret.txt climbs above the workspace root.
        // The gateway rejects any `..` component, so a link that escapes must be
        // rejected here instead of being silently collapsed onto a root-level
        // file of the same name.
        XCTAssertNil(
            GaryxMobileFileLink.previewTarget(
                fromLink: "../../secret.txt",
                workspacePaths: ["/workspace/project"],
                currentWorkspaceDir: "/workspace/project",
                currentFilePath: "docs/readme.md"
            )
        )
        // Escapes from a root-level file.
        XCTAssertNil(
            GaryxMobileFileLink.previewTarget(
                fromLink: "../escape.md",
                workspacePaths: ["/workspace/project"],
                currentWorkspaceDir: "/workspace/project",
                currentFilePath: "readme.md"
            )
        )
        // Escapes with no current-file context.
        XCTAssertNil(
            GaryxMobileFileLink.previewTarget(
                fromLink: "../sibling-project/file.md",
                workspacePaths: ["/workspace/project"],
                currentWorkspaceDir: "/workspace/project",
                currentFilePath: nil
            )
        )
        // Deep escapes are rejected as well.
        XCTAssertNil(
            GaryxMobileFileLink.previewTarget(
                fromLink: "../../../etc/passwd",
                workspacePaths: ["/workspace/project"],
                currentWorkspaceDir: "/workspace/project",
                currentFilePath: "docs/notes/readme.md"
            )
        )
    }

    func testMobileFileLinkKeepsInBoundsParentTraversalWorking() {
        // A link that folds through `..` but stays inside the workspace is valid.
        XCTAssertEqual(
            GaryxMobileFileLink.previewTarget(
                fromLink: "docs/../readme.md",
                workspacePaths: ["/workspace/project"],
                currentWorkspaceDir: "/workspace/project",
                currentFilePath: nil
            ),
            GaryxMobileWorkspaceFileTarget(
                workspaceDir: "/workspace/project",
                path: "readme.md"
            )
        )
        // Traversal up to exactly the workspace root stays in bounds.
        XCTAssertEqual(
            GaryxMobileFileLink.previewTarget(
                fromLink: "../../index.html",
                workspacePaths: ["/workspace/project"],
                currentWorkspaceDir: "/workspace/project",
                currentFilePath: "docs/notes/readme.md"
            ),
            GaryxMobileWorkspaceFileTarget(
                workspaceDir: "/workspace/project",
                path: "index.html"
            )
        )
    }

    // MARK: - Shared HTTP retry semantics (JSON + text routes)

    func testGatewayClientTextRouteRetriesTransientServerErrors() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let attemptCount = GaryxAtomicCounter()
        GaryxURLProtocolStub.requestHandler = { request in
            let attempt = attemptCount.increment()
            let statusCode = attempt < 3 ? 503 : 200
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: statusCode,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "text/html; charset=utf-8"]
                )
            )
            let body = statusCode == 200 ? "<html>ok</html>" : "unavailable"
            return (response, Data(body.utf8))
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: GaryxGatewayRetryPolicy(
                maxAttempts: 3,
                initialDelay: 0.01,
                maxDelay: 0.01,
                backoffMultiplier: 1.0,
                jitter: 0
            )
        )

        let html = try await client.capsuleHTML(id: "cap-1")
        XCTAssertEqual(html, "<html>ok</html>")
        XCTAssertEqual(attemptCount.value(), 3)
    }

    func testGatewayClientTextRouteCanDisableInnerRetryAndCarriesRetryAfter() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let attemptCount = GaryxAtomicCounter()
        GaryxURLProtocolStub.requestHandler = { request in
            _ = attemptCount.increment()
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 429,
                    httpVersion: nil,
                    headerFields: ["Retry-After": "9"]
                )
            )
            return (response, Data("rate limited".utf8))
        }
        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: GaryxGatewayRetryPolicy(
                maxAttempts: 4,
                initialDelay: 0,
                maxDelay: 0,
                jitter: 0
            )
        )

        do {
            _ = try await client.capsuleHTML(id: "capsule-1", allowsRetry: false)
            XCTFail("Expected the single attempt to surface 429")
        } catch let error as GaryxGatewayError {
            guard case let .httpStatus(status, _, retryAfter) = error else {
                return XCTFail("Expected structured HTTP failure, got \(error)")
            }
            XCTAssertEqual(status, 429)
            XCTAssertEqual(retryAfter, 9)
            XCTAssertEqual(attemptCount.value(), 1)
        }
    }

    func testGatewayClientTextRouteDoesNotRetryClientErrors() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let attemptCount = GaryxAtomicCounter()
        GaryxURLProtocolStub.requestHandler = { request in
            _ = attemptCount.increment()
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 404,
                    httpVersion: nil,
                    headerFields: nil
                )
            )
            return (response, Data("missing".utf8))
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: GaryxGatewayRetryPolicy(
                maxAttempts: 3,
                initialDelay: 0.01,
                maxDelay: 0.01,
                backoffMultiplier: 1.0,
                jitter: 0
            )
        )

        do {
            _ = try await client.capsuleHTML(id: "cap-1")
            XCTFail("Expected 404 to surface without retry")
        } catch let error as GaryxGatewayError {
            guard case .httpStatus(let code, _, _) = error else {
                XCTFail("Expected httpStatus error, got \(error)")
                return
            }
            XCTAssertEqual(code, 404)
            XCTAssertEqual(attemptCount.value(), 1)
        }
    }

    func testGatewayClientTextRouteSurfacesNonUTF8WithoutRetry() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let attemptCount = GaryxAtomicCounter()
        GaryxURLProtocolStub.requestHandler = { request in
            _ = attemptCount.increment()
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "text/html"]
                )
            )
            return (response, Data([0xFF, 0xFE, 0xFD]))
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: GaryxGatewayRetryPolicy(
                maxAttempts: 3,
                initialDelay: 0.01,
                maxDelay: 0.01,
                backoffMultiplier: 1.0,
                jitter: 0
            )
        )

        do {
            _ = try await client.capsuleHTML(id: "cap-1")
            XCTFail("Expected non-UTF-8 body to surface encodingFailed without retry")
        } catch let error as GaryxGatewayError {
            guard case .encodingFailed = error else {
                XCTFail("Expected encodingFailed error, got \(error)")
                return
            }
            XCTAssertEqual(attemptCount.value(), 1)
        }
    }

    func testGatewayClientTextRouteRetriesConnectionEstablishmentErrors() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let attemptCount = GaryxAtomicCounter()
        GaryxURLProtocolStub.requestHandler = { request in
            let attempt = attemptCount.increment()
            if attempt == 1 {
                throw URLError(.cannotConnectToHost)
            }
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "text/html"]
                )
            )
            return (response, Data("<html>ok</html>".utf8))
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: GaryxGatewayRetryPolicy(
                maxAttempts: 3,
                initialDelay: 0.01,
                maxDelay: 0.01,
                backoffMultiplier: 1.0,
                jitter: 0
            )
        )

        let html = try await client.capsuleHTML(id: "cap-1")
        XCTAssertEqual(html, "<html>ok</html>")
        XCTAssertEqual(attemptCount.value(), 2)
    }

    func testGatewayClientJSONRouteSurfacesDecodeFailureWithoutRetry() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let attemptCount = GaryxAtomicCounter()
        GaryxURLProtocolStub.requestHandler = { request in
            _ = attemptCount.increment()
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (response, Data("not json".utf8))
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: GaryxGatewayRetryPolicy(
                maxAttempts: 3,
                initialDelay: 0.01,
                maxDelay: 0.01,
                backoffMultiplier: 1.0,
                jitter: 0
            )
        )

        do {
            _ = try await client.status()
            XCTFail("Expected decode failure to surface without retry")
        } catch is DecodingError {
            XCTAssertEqual(attemptCount.value(), 1)
        }
    }

    func testGatewayClientHonorsRetryAfterHeaderOnRetryableStatus() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let attemptCount = GaryxAtomicCounter()
        GaryxURLProtocolStub.requestHandler = { request in
            let attempt = attemptCount.increment()
            let statusCode = attempt < 2 ? 429 : 200
            var headers = ["Content-Type": "application/json"]
            if statusCode == 429 {
                headers["Retry-After"] = "0"
            }
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: statusCode,
                    httpVersion: nil,
                    headerFields: headers
                )
            )
            let body = statusCode == 200 ? #"{"status":"ok"}"# : #"{"error":"slow down"}"#
            return (response, Data(body.utf8))
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: GaryxGatewayRetryPolicy(
                maxAttempts: 3,
                initialDelay: 0.01,
                maxDelay: 0.01,
                backoffMultiplier: 1.0,
                jitter: 0
            )
        )

        let status = try await client.status()
        XCTAssertEqual(status.status, "ok")
        XCTAssertEqual(attemptCount.value(), 2)
    }

    func testGatewayClientPropagatesCancellationDuringRetryDelay() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let attemptCount = GaryxAtomicCounter()
        GaryxURLProtocolStub.requestHandler = { request in
            _ = attemptCount.increment()
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 503,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (response, Data(#"{"error":"unavailable"}"#.utf8))
        }

        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: GaryxGatewayRetryPolicy(
                maxAttempts: 3,
                initialDelay: 5.0,
                maxDelay: 5.0,
                backoffMultiplier: 1.0,
                jitter: 0
            )
        )

        let task = Task { try await client.status() }
        while attemptCount.value() < 1 {
            try await Task.sleep(nanoseconds: 5_000_000)
        }
        // Give the retry loop a beat to enter the retry delay, then cancel.
        try await Task.sleep(nanoseconds: 20_000_000)
        task.cancel()

        do {
            _ = try await task.value
            XCTFail("Expected cancellation to propagate out of the retry delay")
        } catch {
            XCTAssertTrue(
                error is CancellationError || GaryxGatewayRetryClassifier.isCancellation(error),
                "Expected a cancellation error, got \(error)"
            )
            XCTAssertEqual(attemptCount.value(), 1)
        }
    }

    func testAvatarGenerationDoesNotRetryProviderFailure() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxURLProtocolStub.self]
        let session = URLSession(configuration: configuration)
        defer {
            GaryxURLProtocolStub.requestHandler = nil
            session.invalidateAndCancel()
        }

        let attemptCount = GaryxAtomicCounter()
        GaryxURLProtocolStub.requestHandler = { request in
            _ = attemptCount.increment()
            let response = try XCTUnwrap(
                HTTPURLResponse(
                    url: try XCTUnwrap(request.url),
                    statusCode: 502,
                    httpVersion: nil,
                    headerFields: ["Content-Type": "application/json"]
                )
            )
            return (response, Data(#"{"error":"provider failed"}"#.utf8))
        }
        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: GaryxGatewayRetryPolicy(
                maxAttempts: 3,
                initialDelay: 0,
                maxDelay: 0,
                jitter: 0
            )
        )

        do {
            _ = try await client.generateAvatar(prompt: "avatar")
            XCTFail("Expected provider failure")
        } catch GaryxGatewayError.httpStatus(let status, _, _) {
            XCTAssertEqual(status, 502)
            XCTAssertEqual(attemptCount.value(), 1)
        }
    }

    func testAvatarGenerationTaskCancellationStopsURLSessionRequest() async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [GaryxBlockingURLProtocol.self]
        let session = URLSession(configuration: configuration)
        let started = expectation(description: "avatar request started")
        let stopped = expectation(description: "avatar request stopped")
        GaryxBlockingURLProtocol.configure(
            onStart: { started.fulfill() },
            onStop: { stopped.fulfill() }
        )
        defer {
            GaryxBlockingURLProtocol.reset()
            session.invalidateAndCancel()
        }
        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/"))
            ),
            session: session,
            retryPolicy: .disabled
        )
        let task = Task {
            try await client.generateAvatar(prompt: "cancellable avatar")
        }

        await fulfillment(of: [started], timeout: 2)
        task.cancel()
        do {
            _ = try await task.value
            XCTFail("Expected avatar request cancellation")
        } catch {
            XCTAssertTrue(GaryxGatewayRetryClassifier.isCancellation(error))
        }
        await fulfillment(of: [stopped], timeout: 2)
    }
}

private func garyxRequestBodyData(from request: URLRequest) -> Data? {
    if let body = request.httpBody {
        return body
    }
    guard let stream = request.httpBodyStream else {
        return nil
    }
    stream.open()
    defer { stream.close() }

    var data = Data()
    let bufferSize = 4096
    let buffer = UnsafeMutablePointer<UInt8>.allocate(capacity: bufferSize)
    defer { buffer.deallocate() }

    while stream.hasBytesAvailable {
        let count = stream.read(buffer, maxLength: bufferSize)
        if count > 0 {
            data.append(buffer, count: count)
        } else {
            break
        }
    }
    return data
}

private final class GaryxURLProtocolStub: URLProtocol {
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

private final class GaryxBlockingURLProtocol: URLProtocol {
    private static let lock = NSLock()
    private static var startCallback: (() -> Void)?
    private static var stopCallback: (() -> Void)?

    static func configure(onStart: @escaping () -> Void, onStop: @escaping () -> Void) {
        lock.lock()
        defer { lock.unlock() }
        startCallback = onStart
        stopCallback = onStop
    }

    static func reset() {
        lock.lock()
        defer { lock.unlock() }
        startCallback = nil
        stopCallback = nil
    }

    override class func canInit(with request: URLRequest) -> Bool {
        true
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest {
        request
    }

    override func startLoading() {
        Self.lock.lock()
        let callback = Self.startCallback
        Self.lock.unlock()
        callback?()
    }

    override func stopLoading() {
        Self.lock.lock()
        let callback = Self.stopCallback
        Self.lock.unlock()
        callback?()
    }
}

private final class GaryxAtomicCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var current = 0

    func increment() -> Int {
        lock.lock()
        defer { lock.unlock() }
        current += 1
        return current
    }

    func value() -> Int {
        lock.lock()
        defer { lock.unlock() }
        return current
    }
}
