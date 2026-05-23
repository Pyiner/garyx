import XCTest
@testable import GaryxMobileCore

final class GaryxMobileTurnRendererTests: XCTestCase {
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
        XCTAssertEqual(turn.id, "turn:user-1")
        XCTAssertEqual(turn.steps.map(\.id), ["tool-1"])
        XCTAssertEqual(turn.finalBlock?.id, "assistant-1")
        XCTAssertFalse(turn.isRunning)
        XCTAssertEqual(turn.startedAt, "2026-01-01T00:00:00Z")
        XCTAssertEqual(turn.finishedAt, "2026-01-01T00:00:02Z")
    }

    func testTrailingUserWhileRunningCreatesStableEmptyTurn() throws {
        let rows = GaryxMobileTurnRenderer.buildTurnRows(
            messages: [
                message("user-1", role: .user, text: "Run this"),
            ],
            isRunningThread: true
        )

        let activity = try XCTUnwrap(try XCTUnwrap(rows.only).activityRows.only)
        guard case .turn(let turn) = activity else {
            return XCTFail("Expected pending user turn")
        }
        XCTAssertEqual(turn.id, "turn:user-1")
        XCTAssertTrue(turn.steps.isEmpty)
        XCTAssertNil(turn.finalBlock)
        XCTAssertTrue(turn.isRunning)
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
        timestamp: String? = nil,
        isStreaming: Bool = false
    ) -> GaryxMobileMessage {
        GaryxMobileMessage(
            id: id,
            role: role,
            text: text,
            timestamp: timestamp,
            isStreaming: isStreaming
        )
    }

    private func toolMessage(_ id: String, timestamp: String? = nil) -> GaryxMobileMessage {
        GaryxMobileMessage(
            id: id,
            role: .tool,
            text: "",
            timestamp: timestamp,
            isStreaming: false,
            toolTraceGroup: GaryxMobileToolTraceGroup(
                entries: [
                    toolEntry(
                        id: "\(id)-entry",
                        toolName: "exec_command",
                        title: "Command",
                        status: .completed
                    ),
                ]
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
}

private extension Array {
    var only: Element? {
        count == 1 ? self[0] : nil
    }
}
