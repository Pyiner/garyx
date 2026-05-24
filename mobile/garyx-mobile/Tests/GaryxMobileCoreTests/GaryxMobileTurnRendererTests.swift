import XCTest
@testable import GaryxMobileCore

final class GaryxMobileTurnRendererTests: XCTestCase {
    func testMatchesDesktopTurnRenderParityFixture() throws {
        let fixture = try loadTurnRenderParityFixture()

        for testCase in fixture.cases {
            let rows = GaryxMobileTurnRenderer.buildTurnRows(
                messages: testCase.messages.map(mobileMessage),
                isRunningThread: testCase.isRunningThread
            )

            XCTAssertEqual(snapshot(rows), testCase.expected, testCase.name)
        }
    }

    func testCompletedSingleAssistantAnswerStaysFlat() throws {
        let rows = GaryxMobileTurnRenderer.buildTurnRows(
            messages: [
                message("user-1", role: .user, text: "Question"),
                message("assistant-1", role: .assistant, text: "Answer"),
            ],
            isRunningThread: false
        )

        let row = try XCTUnwrap(rows.only)
        XCTAssertEqual(row.id, "user-turn:user-1")
        XCTAssertEqual(row.userBlock?.id, "user-1")

        guard case .flat(let block) = try XCTUnwrap(row.activityRows.only) else {
            return XCTFail("Expected completed single-answer turn to stay flat")
        }
        XCTAssertEqual(block.id, "assistant-1")
    }

    func testRunningTurnDefersTrailingAssistantAnswerIntoTurnBody() throws {
        let rows = GaryxMobileTurnRenderer.buildTurnRows(
            messages: [
                message("user-1", role: .user, text: "Question"),
                message("assistant-1", role: .assistant, text: "Partial final-looking text"),
            ],
            isRunningThread: true
        )

        let activity = try XCTUnwrap(try XCTUnwrap(rows.only).activityRows.only)
        guard case .turn(let turn) = activity else {
            return XCTFail("Expected running trailing answer to remain inside the turn")
        }
        XCTAssertNil(turn.finalBlock)
        XCTAssertEqual(turn.steps.map(\.id), ["assistant-1"])
    }

    func testCompletedTurnSurfacesTrailingAssistantAfterToolActivity() throws {
        let rows = GaryxMobileTurnRenderer.buildTurnRows(
            messages: [
                message("user-1", role: .user, timestamp: "2026-01-01T00:00:00Z"),
                toolMessage("tool-1", timestamp: "2026-01-01T00:00:01Z"),
                message("assistant-1", role: .assistant, text: "Final answer", timestamp: "2026-01-01T00:00:02Z"),
            ],
            isRunningThread: false
        )

        let activity = try XCTUnwrap(try XCTUnwrap(rows.only).activityRows.only)
        guard case .turn(let turn) = activity else {
            return XCTFail("Expected tool activity to render as a collapsible turn")
        }
        XCTAssertEqual(turn.id, "turn:tool-1")
        XCTAssertEqual(turn.steps.map(\.id), ["tool-1"])
        XCTAssertEqual(turn.finalBlock?.id, "assistant-1")
        XCTAssertFalse(turn.isRunning)
        XCTAssertEqual(turn.startedAt, "2026-01-01T00:00:00Z")
        XCTAssertEqual(turn.finishedAt, "2026-01-01T00:00:02Z")
    }

    func testTrailingUserWhileRunningDoesNotCreateEmptyTurn() throws {
        let rows = GaryxMobileTurnRenderer.buildTurnRows(
            messages: [
                message("user-1", role: .user, text: "Run this"),
            ],
            isRunningThread: true
        )

        let row = try XCTUnwrap(rows.only)
        XCTAssertEqual(row.id, "user-turn:user-1")
        XCTAssertEqual(row.userBlock?.id, "user-1")
        XCTAssertTrue(row.activityRows.isEmpty)
    }

    func testRunningSecondTurnKeepsPriorPureReplyOnFirstTurn() throws {
        let rows = GaryxMobileTurnRenderer.buildTurnRows(
            messages: [
                message("user-1", role: .user, text: "Hello"),
                message("assistant-1", role: .assistant, text: "Hi, I am here."),
                message("user-2", role: .user, text: "Run this"),
                toolMessage("tool-1"),
                message("assistant-2", role: .assistant, text: "Working on it", isStreaming: true),
            ],
            isRunningThread: true
        )

        XCTAssertEqual(rows.map(\.userBlock?.id), ["user-1", "user-2"])
        guard case .flat(let firstReply) = try XCTUnwrap(rows[0].activityRows.only) else {
            return XCTFail("Expected the first completed reply to stay outside the running turn")
        }
        XCTAssertEqual(firstReply.id, "assistant-1")

        guard case .turn(let secondTurn) = try XCTUnwrap(rows[1].activityRows.only) else {
            return XCTFail("Expected the running second turn to render its own activity")
        }
        XCTAssertNil(secondTurn.finalBlock)
        XCTAssertEqual(secondTurn.steps.map(\.id), ["tool-1", "assistant-2"])
        XCTAssertFalse(secondTurn.steps.contains { $0.id == "assistant-1" })
    }

    func testActivityModelShowsThinkingForTrailingUserOnlyRun() {
        let messages = [
            message("user-1", role: .user, text: "Run this"),
        ]

        XCTAssertTrue(GaryxMobileThreadActivityModel.latestUserMessageAwaitsAssistant(messages))
        XCTAssertTrue(
            GaryxMobileThreadActivityModel.showsTailThinkingIndicator(
                messages: messages,
                runActive: true
            )
        )
    }

    func testActivityModelSuppressesThinkingForEmptyAssistantPlaceholderAndActiveTool() {
        XCTAssertFalse(
            GaryxMobileThreadActivityModel.showsTailThinkingIndicator(
                messages: [
                    message("user-1", role: .user, text: "Run this"),
                    message("assistant-1", role: .assistant, isStreaming: true),
                ],
                runActive: true
            )
        )
        XCTAssertFalse(
            GaryxMobileThreadActivityModel.showsTailThinkingIndicator(
                messages: [
                    message("user-1", role: .user, text: "Run this"),
                    toolMessage("tool-1", isActive: true),
                ],
                runActive: true
            )
        )
    }

    func testActivityModelShowsThinkingForImageOnlyStreamingAssistant() {
        XCTAssertTrue(
            GaryxMobileThreadActivityModel.showsTailThinkingIndicator(
                messages: [
                    message("user-1", role: .user, text: "Make an image"),
                    message(
                        "assistant-1",
                        role: .assistant,
                        attachments: [
                            GaryxMobileMessageAttachment(
                                id: "image-1",
                                kind: "image",
                                name: "Image",
                                mediaType: "image/png"
                            ),
                        ],
                        isStreaming: true
                    ),
                ],
                runActive: true
            )
        )
    }

    func testTranscriptMapperDoesNotAppendActiveRunAssistantResponseAfterPendingInput() throws {
        let transcript = try JSONDecoder().decode(
            GaryxThreadTranscript.self,
            from: Data(
                """
                {
                  "ok": true,
                  "messages": [
                    {
                      "index": 0,
                      "role": "user",
                      "text": "Hello",
                      "timestamp": "2026-01-01T00:00:00Z"
                    },
                    {
                      "index": 1,
                      "role": "assistant",
                      "text": "Hi, I am here.",
                      "timestamp": "2026-01-01T00:00:01Z"
                    }
                  ],
                  "pending_user_inputs": [
                    {
                      "id": "pending-1",
                      "run_id": "run-1",
                      "text": "Run this",
                      "timestamp": "2026-01-01T00:00:02Z",
                      "status": "awaiting_ack",
                      "active": true
                    }
                  ],
                  "thread_runtime": {
                    "active_run": {
                      "run_id": "run-1",
                      "assistant_response": "Hi, I am here.",
                      "updated_at": "2026-01-01T00:00:03Z",
                      "pending_user_input_count": 1
                    }
                  }
                }
                """.utf8
            )
        )
        XCTAssertEqual(transcript.threadRuntime?.activeRun?.assistantResponse, "Hi, I am here.")

        let rendered = GaryxMobileTranscriptMapper.appendPendingUserInputs(
            to: [
                message("history:0", role: .user, text: "Hello"),
                message("history:1", role: .assistant, text: "Hi, I am here."),
            ],
            from: transcript
        )

        XCTAssertEqual(rendered.map(\.text), ["Hello", "Hi, I am here.", "Run this"])
        XCTAssertEqual(rendered.filter { $0.role == .assistant && $0.text == "Hi, I am here." }.count, 1)
        XCTAssertEqual(rendered.last?.pendingInputId, "pending-1")
    }

    func testOrphanAssistantTurnUsesStableMessageId() throws {
        let messages = [
            message("assistant-1", role: .assistant, text: "Restored answer"),
        ]

        let first = GaryxMobileTurnRenderer.buildTurnRows(messages: messages, isRunningThread: false)
        let second = GaryxMobileTurnRenderer.buildTurnRows(messages: messages, isRunningThread: false)

        XCTAssertEqual(first.map(\.id), second.map(\.id))
        XCTAssertEqual(try XCTUnwrap(first.only).id, "orphan-turn:assistant-1")
        XCTAssertNil(try XCTUnwrap(first.only).userBlock)
    }

    func testTurnIdsStayStableWhenMessageTextChanges() throws {
        let original = GaryxMobileTurnRenderer.buildTurnRows(
            messages: [
                message("user-1", role: .user, text: "Question"),
                toolMessage("tool-1"),
                message("assistant-1", role: .assistant, text: "Answer"),
            ],
            isRunningThread: false
        )
        let updated = GaryxMobileTurnRenderer.buildTurnRows(
            messages: [
                message("user-1", role: .user, text: "Edited question"),
                toolMessage("tool-1"),
                message("assistant-1", role: .assistant, text: "Edited answer"),
            ],
            isRunningThread: false
        )

        XCTAssertEqual(original.map(\.id), updated.map(\.id))
        XCTAssertEqual(
            try XCTUnwrap(original.only).activityRows.map(\.id),
            try XCTUnwrap(updated.only).activityRows.map(\.id)
        )
    }

    func testToolTraceAbsorbPromotesResultData() {
        var entry = toolEntry(
            id: "use-1",
            toolUseId: nil,
            toolName: "tool",
            title: "Tool",
            status: .running,
            inputText: "input"
        )
        let result = toolEntry(
            id: "result-1",
            toolUseId: "tool-use-1",
            toolName: "apply_patch",
            title: "Patch",
            status: .completed,
            resultText: "patched",
            summaryText: "Updated one file",
            primaryPathBadge: "App.swift"
        )

        entry.absorb(result: result)

        XCTAssertEqual(entry.toolUseId, "tool-use-1")
        XCTAssertEqual(entry.toolName, "apply_patch")
        XCTAssertEqual(entry.title, "Patch")
        XCTAssertEqual(entry.resultText, "patched")
        XCTAssertEqual(entry.summaryText, "Updated one file")
        XCTAssertEqual(entry.status, .completed)
        XCTAssertEqual(entry.primaryPathBadge, "App.swift")
    }

    func testToolTraceGroupSummaryCountsCommandsAndEditedFiles() {
        let group = GaryxMobileToolTraceGroup(
            entries: [
                toolEntry(id: "command-1", toolName: "exec_command", title: "Command", status: .completed),
                toolEntry(
                    id: "edit-1",
                    toolName: "apply_patch",
                    title: "Patch",
                    status: .completed,
                    primaryPathBadge: "App.swift"
                ),
                toolEntry(
                    id: "edit-2",
                    toolName: "edit",
                    title: "Edit",
                    status: .completed,
                    primaryPathBadge: "App.swift"
                ),
            ]
        )

        XCTAssertEqual(group.summary, "Edited 1 file, Ran 1 command")
    }

    private func message(
        _ id: String,
        role: GaryxMobileMessage.Role,
        text: String = "",
        attachments: [GaryxMobileMessageAttachment] = [],
        timestamp: String? = nil,
        isStreaming: Bool = false
    ) -> GaryxMobileMessage {
        GaryxMobileMessage(
            id: id,
            role: role,
            text: text,
            attachments: attachments,
            timestamp: timestamp,
            isStreaming: isStreaming
        )
    }

    private func toolMessage(
        _ id: String,
        timestamp: String? = nil,
        isActive: Bool = false
    ) -> GaryxMobileMessage {
        GaryxMobileMessage(
            id: id,
            role: .tool,
            text: "",
            timestamp: timestamp,
            isStreaming: isActive,
            toolTraceGroup: GaryxMobileToolTraceGroup(
                entries: [
                    toolEntry(
                        id: "\(id)-entry",
                        toolName: "exec_command",
                        title: "Command",
                        status: isActive ? .running : .completed
                    ),
                ],
                live: isActive
            )
        )
    }

    private func toolEntry(
        id: String,
        toolUseId: String? = nil,
        toolName: String,
        title: String,
        status: GaryxMobileToolTraceStatus,
        inputText: String? = nil,
        resultText: String? = nil,
        summaryText: String? = nil,
        primaryPathBadge: String? = nil
    ) -> GaryxMobileToolTraceEntry {
        GaryxMobileToolTraceEntry(
            id: id,
            toolUseId: toolUseId,
            parentToolUseId: nil,
            toolName: toolName,
            title: title,
            inputText: inputText,
            resultText: resultText,
            summaryText: summaryText,
            inputLabel: "Input",
            resultLabel: "Result",
            status: status,
            isError: false,
            timestamp: nil,
            primaryPathBadge: primaryPathBadge
        )
    }

    private func mobileMessage(_ fixture: TurnRenderParityMessage) -> GaryxMobileMessage {
        switch fixture.role {
        case "user":
            return message(
                fixture.id,
                role: .user,
                text: fixture.text ?? "",
                timestamp: fixture.timestamp,
                isStreaming: fixture.isStreaming ?? false
            )
        case "assistant":
            return message(
                fixture.id,
                role: .assistant,
                text: fixture.text ?? "",
                timestamp: fixture.timestamp,
                isStreaming: fixture.isStreaming ?? false
            )
        case "tool":
            return toolMessage(fixture.id, timestamp: fixture.timestamp)
        default:
            XCTFail("Unknown fixture role \(fixture.role)")
            return message(fixture.id, role: .system, text: fixture.text ?? "", timestamp: fixture.timestamp)
        }
    }

    private func snapshot(_ rows: [GaryxMobileTurnRow]) -> [TurnRenderParitySnapshot] {
        rows.map { row in
            TurnRenderParitySnapshot(
                kind: "user_turn",
                key: row.id,
                user: row.userBlock?.id,
                activity: row.activityRows.map(snapshot)
            )
        }
    }

    private func snapshot(_ row: GaryxMobileTurnRow.ActivityRow) -> TurnRenderParitySnapshot {
        switch row {
        case .flat(let block):
            return TurnRenderParitySnapshot(kind: "flat", key: block.id)
        case .turn(let turn):
            return TurnRenderParitySnapshot(
                kind: "turn",
                key: turn.id,
                steps: turn.steps.map(\.id),
                final: turn.finalBlock?.id,
                running: turn.isRunning,
                startedAt: turn.startedAt,
                finishedAt: turn.finishedAt
            )
        }
    }
}

private struct TurnRenderParityFixture: Decodable {
    var cases: [TurnRenderParityCase]
}

private struct TurnRenderParityCase: Decodable {
    var name: String
    var isRunningThread: Bool
    var messages: [TurnRenderParityMessage]
    var expected: [TurnRenderParitySnapshot]
}

private struct TurnRenderParityMessage: Decodable {
    var id: String
    var role: String
    var text: String?
    var timestamp: String?
    var isStreaming: Bool?
}

private struct TurnRenderParitySnapshot: Decodable, Equatable {
    var kind: String
    var key: String
    var user: String?
    var activity: [TurnRenderParitySnapshot]?
    var steps: [String]?
    var final: String?
    var running: Bool?
    var startedAt: String?
    var finishedAt: String?

    init(
        kind: String,
        key: String,
        user: String? = nil,
        activity: [TurnRenderParitySnapshot]? = nil,
        steps: [String]? = nil,
        final: String? = nil,
        running: Bool? = nil,
        startedAt: String? = nil,
        finishedAt: String? = nil
    ) {
        self.kind = kind
        self.key = key
        self.user = user
        self.activity = activity
        self.steps = steps
        self.final = final
        self.running = running
        self.startedAt = startedAt
        self.finishedAt = finishedAt
    }
}

private func loadTurnRenderParityFixture() throws -> TurnRenderParityFixture {
    var url = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 {
        url.deleteLastPathComponent()
    }
    url.appendPathComponent("test-fixtures/turn-render-parity.json")
    let data = try Data(contentsOf: url)
    return try JSONDecoder().decode(TurnRenderParityFixture.self, from: data)
}

private extension Array {
    var only: Element? {
        count == 1 ? self[0] : nil
    }
}
