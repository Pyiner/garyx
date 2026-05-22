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

    func testMobileConnectLinkRoundTripsGatewaySettings() throws {
        let url = try XCTUnwrap(
            GaryxMobileConnectLink.make(
                gatewayUrl: "http://192.168.1.20:31337",
                gatewayAuthToken: "test gateway token"
            )
        )

        let payload = try XCTUnwrap(GaryxMobileConnectLink.parse(url))

        XCTAssertEqual(payload.gatewayUrl, "http://192.168.1.20:31337")
        XCTAssertEqual(payload.gatewayAuthToken, "test gateway token")
    }

    func testMobileConnectLinkAcceptsTokenAlias() throws {
        let url = try XCTUnwrap(
            URL(string: "garyx://connect?url=http%3A%2F%2F192.168.1.20%3A31337&token=test-token")
        )

        let payload = try XCTUnwrap(GaryxMobileConnectLink.parse(url))

        XCTAssertEqual(payload.gatewayUrl, "http://192.168.1.20:31337")
        XCTAssertEqual(payload.gatewayAuthToken, "test-token")
    }

    func testWebSocketURLCarriesGatewayTokenInQuery() throws {
        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "https://gateway.example.test/")),
                authToken: "test token"
            )
        )

        let url = try client.chatWebSocketURL()

        XCTAssertEqual(url.scheme, "wss")
        XCTAssertEqual(url.host(), "gateway.example.test")
        XCTAssertEqual(url.path(), "/api/chat/ws")
        XCTAssertEqual(
            URLComponents(url: url, resolvingAgainstBaseURL: false)?
                .queryItems?
                .first(where: { $0.name == "token" })?
                .value,
            "test token"
        )
    }

    func testEventStreamRequestUsesExistingGatewaySSEEndpoint() throws {
        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://gateway.example.test/garyx")),
                authToken: "test token"
            )
        )

        let request = try client.eventStreamRequest(historyLimit: 20)

        XCTAssertEqual(request.httpMethod, "GET")
        XCTAssertEqual(request.url?.path(), "/garyx/api/stream")
        XCTAssertEqual(
            URLComponents(url: try XCTUnwrap(request.url), resolvingAgainstBaseURL: false)?
                .queryItems?
                .first(where: { $0.name == "history_limit" })?
                .value,
            "20"
        )
        XCTAssertEqual(request.value(forHTTPHeaderField: "Accept"), "text/event-stream")
        XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer test token")
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

    func testStartCommandEncodesGatewayChatOperation() throws {
        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://127.0.0.1:31337"))
            )
        )

        let text = try client.encodeWebSocketCommand(
            .start(
                threadId: "thread::test",
                message: "hello",
                workspacePath: "/path/to/repo",
                metadata: ["client": "garyx-mobile"]
            )
        )
        let object = try JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any]

        XCTAssertEqual(object?["op"] as? String, "start")
        XCTAssertEqual(object?["threadId"] as? String, "thread::test")
        XCTAssertEqual(object?["message"] as? String, "hello")
        XCTAssertEqual(object?["waitForResponse"] as? Bool, false)
        XCTAssertEqual(object?["workspacePath"] as? String, "/path/to/repo")
        XCTAssertEqual((object?["metadata"] as? [String: String])?["client"], "garyx-mobile")
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

    func testThreadSummaryDecodesTeamHints() throws {
        let data = Data(
            """
            {
              "thread_id": "thread::team",
              "team_id": "team-alpha",
              "team_display_name": "Alpha Team",
              "last_assistant_message": "ready"
            }
            """.utf8
        )

        let summary = try JSONDecoder().decode(GaryxThreadSummary.self, from: data)

        XCTAssertEqual(summary.id, "thread::team")
        XCTAssertEqual(summary.teamId, "team-alpha")
        XCTAssertEqual(summary.teamName, "Alpha Team")
        XCTAssertEqual(summary.lastMessagePreview, "ready")
    }

    func testThreadPinsPageDecodesGatewayShape() throws {
        let page = try JSONDecoder().decode(
            GaryxThreadPinsPage.self,
            from: Data(
                """
                {
                  "thread_ids": ["thread::one", " thread::two ", "thread::one", ""]
                }
                """.utf8
            )
        )

        XCTAssertEqual(page.threadIds, ["thread::one", "thread::two"])
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
        let teams = try JSONDecoder().decode(
            GaryxTeamsPage.self,
            from: Data(
                """
                {
                  "teams": [
                    {
                      "team_id": "team-alpha",
                      "display_name": "Alpha Team",
                      "leader_agent_id": "codex",
                      "member_agent_ids": ["codex", "claude"],
                      "workflow_text": "Plan, implement, review."
                    }
                  ]
                }
                """.utf8
            )
        )
        let tasks = try JSONDecoder().decode(
            GaryxTasksPage.self,
            from: Data(
                """
                {
                  "tasks": [
                    {
                      "thread_id": "thread::task",
                      "task_id": "task::1",
                      "number": 1,
                      "title": "Ship mobile parity",
                      "status": "in_progress",
                      "creator": { "kind": "human", "user_id": "test-user" },
                      "assignee": { "kind": "agent", "agent_id": "codex" },
                      "source": {
                        "thread_id": "thread::source",
                        "task_thread_id": "thread::task",
                        "bot_id": "bot-test",
                        "channel": "api",
                        "account_id": "account-test"
                      },
                      "updated_at": "2026-03-01T09:30:00Z",
                      "updated_by": { "kind": "agent", "agent_id": "codex" },
                      "runtime_agent_id": "codex",
                      "reply_count": 2
                    }
                  ],
                  "total": 1,
                  "has_more": false
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
        XCTAssertEqual(teams.teams.first?.memberAgentIds, ["codex", "claude"])
        XCTAssertEqual(tasks.tasks.first?.status, .inProgress)
        XCTAssertEqual(tasks.tasks.first?.assigneeLabel, "codex")
        XCTAssertEqual(tasks.tasks.first?.creator?.userId, "test-user")
        XCTAssertEqual(tasks.tasks.first?.assignee?.agentId, "codex")
        XCTAssertEqual(tasks.tasks.first?.source?.channel, "api")
        XCTAssertEqual(tasks.tasks.first?.source?.accountId, "account-test")
        XCTAssertEqual(tasks.tasks.first?.updatedAt, "2026-03-01T09:30:00Z")
        XCTAssertEqual(tasks.tasks.first?.updatedBy?.agentId, "codex")
        XCTAssertEqual(automations.automations.first?.workspacePath, "/workspace/project")
        XCTAssertEqual(skills.skills.first?.name, "Mobile Skill")
    }

    func testTaskCreateRequestEncodesGatewayShape() throws {
        let request = GaryxTaskCreateRequest(
            title: "Ship mobile parity",
            body: "Synthetic task body.",
            assignee: .agent("codex"),
            start: true,
            runtime: GaryxTaskRuntimeRequest(
                agentId: "codex",
                workspaceDir: "/workspace/project"
            )
        )

        let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any]
        let assignee = object?["assignee"] as? [String: Any]
        let runtime = object?["runtime"] as? [String: Any]
        let notificationTarget = object?["notification_target"] as? [String: Any]

        XCTAssertEqual(object?["title"] as? String, "Ship mobile parity")
        XCTAssertEqual(assignee?["kind"] as? String, "agent")
        XCTAssertEqual(assignee?["agent_id"] as? String, "codex")
        XCTAssertEqual(runtime?["agent_id"] as? String, "codex")
        XCTAssertEqual(runtime?["workspace_dir"] as? String, "/workspace/project")
        XCTAssertEqual(notificationTarget?["kind"] as? String, "none")
    }

    func testTaskCreateResponseMergesEnvelopeAndNestedTask() throws {
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

    func testStreamEventDecodesAssistantDelta() throws {
        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://127.0.0.1:31337"))
            )
        )

        let event = try client.decodeStreamEvent(
            """
            {
              "type": "assistant_delta",
              "runId": "run-test",
              "threadId": "thread::test",
              "delta": "hello",
              "metadata": { "source": "unit-test" }
            }
            """
        )

        XCTAssertEqual(
            event,
            .assistantDelta(
                runId: "run-test",
                threadId: "thread::test",
                delta: "hello",
                metadata: ["source": .string("unit-test")]
            )
        )
    }

    func testStreamEventDecodesUserMessage() throws {
        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://127.0.0.1:31337"))
            )
        )

        let event = try client.decodeStreamEvent(
            """
            {
              "type": "user_message",
              "run_id": "run-test",
              "thread_id": "thread::test",
              "text": "follow up",
              "image_count": 2
            }
            """
        )

        XCTAssertEqual(
            event,
            .userMessage(
                runId: "run-test",
                threadId: "thread::test",
                text: "follow up",
                imageCount: 2
            )
        )
    }

    func testStreamEventDecodesRunComplete() throws {
        let client = GaryxGatewayClient(
            configuration: GaryxGatewayConfiguration(
                baseURL: try XCTUnwrap(URL(string: "http://127.0.0.1:31337"))
            )
        )

        let event = try client.decodeStreamEvent(
            """
            {
              "type": "run_complete",
              "run_id": "run-test",
              "thread_id": "thread::test"
            }
            """
        )

        XCTAssertEqual(event, .runComplete(runId: "run-test", threadId: "thread::test"))
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
        let research = try JSONDecoder().decode(
            GaryxAutoResearchRunsPage.self,
            from: Data(
                """
                {
                  "items": [
                    {
                      "run_id": "research-test",
                      "state": "running",
                      "goal": "Find a safe implementation path.",
                      "workspace_dir": "/workspace/project",
                      "max_iterations": 3,
                      "time_budget_secs": 1200,
                      "iterations_used": 1,
                      "created_at": "2026-03-01T09:00:00Z",
                      "updated_at": "2026-03-01T09:05:00Z"
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
        XCTAssertEqual(workspace.entries.first?.path, "Sources/App.swift")
        XCTAssertTrue(gitStatus.canUseWorktree)
        XCTAssertEqual(gitStatus.currentBranch, "main")
        XCTAssertEqual(commands.commands.first?.prompt, "Finish and verify.")
        XCTAssertEqual(mcp.servers.first?.args, ["server.js"])
        XCTAssertEqual(research.items.first?.runId, "research-test")
        XCTAssertEqual(bots.bots.first?.boundEndpointCount, 1)
        XCTAssertEqual(bots.bots.first?.mainThreadId, "thread::main")
    }

    func testMacParityRequestsEncodeGatewayShapes() throws {
        let automation = GaryxAutomationCreateRequest(
            label: "Interval Review",
            prompt: "Review open work.",
            agentId: "codex",
            workspaceDir: "/workspace/project",
            schedule: .interval(hours: 6),
            enabled: true
        )
        let automationObject = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(automation)
        ) as? [String: Any]
        let automationSchedule = automationObject?["schedule"] as? [String: Any]

        XCTAssertEqual(automationObject?["agentId"] as? String, "codex")
        XCTAssertEqual(automationObject?["workspaceDir"] as? String, "/workspace/project")
        XCTAssertEqual(automationSchedule?["kind"] as? String, "interval")
        XCTAssertEqual(automationSchedule?["hours"] as? Int, 6)

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

        let research = GaryxAutoResearchCreateRequest(
            goal: "Find a safe implementation path.",
            workspaceDir: "/workspace/project",
            maxIterations: 3,
            timeBudgetSecs: 1200
        )
        let researchObject = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(research)
        ) as? [String: Any]

        XCTAssertEqual(researchObject?["workspace_dir"] as? String, "/workspace/project")
        XCTAssertEqual(researchObject?["max_iterations"] as? Int, 3)
        XCTAssertEqual(researchObject?["time_budget_secs"] as? Int, 1200)

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

    func testMacParityAgentTeamAndChannelPayloadsDecodeGatewayShapes() throws {
        let agents = try JSONDecoder().decode(
            GaryxAgentsPage.self,
            from: Data(
                """
                {
                  "agents": [
                    {
                      "agent_id": "agent-test",
                      "display_name": "Test Agent",
                      "provider_type": "codex_app_server",
                      "model": "gpt-test",
                      "model_reasoning_effort": "medium",
                      "model_service_tier": "default",
                      "provider_env": { "TOKEN": "${TOKEN}" },
                      "auth_source": "gateway",
                      "base_url": "https://gateway.example.test",
                      "codex_home": "/workspace/garyx-home",
                      "max_tool_iterations": 12,
                      "request_timeout_seconds": 300,
                      "default_workspace_dir": "/workspace/project",
                      "avatar_data_url": "data:image/png;base64,dGVzdA==",
                      "system_prompt": "Help with test work.",
                      "built_in": false,
                      "standalone": true,
                      "created_at": "2026-03-01T09:00:00Z",
                      "updated_at": "2026-03-01T09:10:00Z"
                    },
                    {
                      "agentId": "agent-remote-avatar",
                      "displayName": "Remote Avatar Agent",
                      "providerType": "codex_app_server",
                      "avatarURL": "https://example.test/avatar.png"
                    }
                  ]
                }
                """.utf8
            )
        )
        let teams = try JSONDecoder().decode(
            GaryxTeamsPage.self,
            from: Data(
                """
                {
                  "teams": [
                    {
                      "team_id": "team-test",
                      "display_name": "Test Team",
                      "leader_agent_id": "agent-test",
                      "member_agent_ids": ["agent-test", "codex"],
                      "workflow_text": "Plan then verify.",
                      "avatar_url": "https://example.test/team-avatar.png",
                      "created_at": "2026-03-01T09:00:00Z"
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
        XCTAssertEqual(agents.agents.first?.systemPrompt, "Help with test work.")
        XCTAssertEqual(agents.agents.last?.avatarDataUrl, "https://example.test/avatar.png")
        XCTAssertEqual(teams.teams.first?.avatarDataUrl, "https://example.test/team-avatar.png")
        XCTAssertEqual(plugins.plugins.first?.iconDataUrl, "data:image/png;base64,dGVzdA==")
        XCTAssertEqual(plugins.plugins.first?.configMethods.first?.kind, "auth_flow")
        XCTAssertEqual(configuredBots.bots.first?.agentId, "agent-test")
        XCTAssertEqual(configuredBots.bots.first?.mainThreadId, nil)
        XCTAssertEqual(botConsoles.bots.first?.agentId, "agent-test")
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
        let candidates = try JSONDecoder().decode(
            GaryxAutoResearchCandidatesPage.self,
            from: Data(
                """
                {
                  "run_id": "research-test",
                  "best_candidate_id": "candidate-test",
                  "selected_candidate": null,
                  "candidates": [
                    {
                      "candidate_id": "candidate-test",
                      "iteration": 1,
                      "thread_id": "thread::test",
                      "output": "Candidate output.",
                      "verdict": { "score": 8.5, "feedback": "Good candidate." }
                    }
                  ]
                }
                """.utf8
            )
        )

        XCTAssertEqual(upload.files.first?.mediaType, "image/png")
        XCTAssertEqual(logs.cursor, 14)
        XCTAssertEqual(candidates.candidates.first?.verdict?.score, 8.5)

        let command = GaryxChatWebSocketCommand.start(
            threadId: "thread::test",
            message: "Review this",
            attachments: [
                GaryxPromptAttachment(
                    kind: "image",
                    path: "/workspace/tmp/prompt-image.png",
                    name: "prompt-image.png",
                    mediaType: "image/png"
                )
            ]
        )
        let commandObject = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(command)
        ) as? [String: Any]
        let attachments = commandObject?["attachments"] as? [[String: Any]]

        XCTAssertEqual(attachments?.first?["media_type"] as? String, "image/png")

        let botRequest = GaryxBotBindingRequest(botId: "telegram:main", threadId: "thread::test")
        let botObject = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(botRequest)
        ) as? [String: Any]

        XCTAssertEqual(botObject?["botId"] as? String, "telegram:main")
        XCTAssertEqual(botObject?["threadId"] as? String, "thread::test")
    }
}
