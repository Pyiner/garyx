import XCTest
@testable import GaryxMobileCore

final class GaryxMobileToolTraceBuilderTests: XCTestCase {
    func testMessagePoolSkipsCommittedToolRows() {
        let messages = GaryxMobileTranscriptMapper.mobileMessages(from: [
            GaryxTranscriptMessage(index: 1, role: .assistant, text: "Done."),
            GaryxTranscriptMessage(
                index: 2,
                role: .toolResult,
                content: json(#"{"result":{"text":"orphan result"}}"#)
            ),
            GaryxTranscriptMessage(index: 3, role: .tool, text: "raw tool output"),
        ])

        XCTAssertEqual(messages.map(\.role), [.assistant])
        XCTAssertTrue(messages.filter { $0.role == .tool }.isEmpty)
    }

    func testMessagePoolDoesNotExposeToolUseRowsAsPlainMessages() {
        let messages = GaryxMobileTranscriptMapper.mobileMessages(from: [
            GaryxTranscriptMessage(index: 1, role: .assistant, text: "Working."),
            GaryxTranscriptMessage(
                index: 2,
                role: .toolUse,
                content: json(#"{"type":"fileChange","path":"/tmp/App.swift"}"#)
            ),
            GaryxTranscriptMessage(
                index: 3,
                role: .toolUse,
                content: json(#"{"type":"contextCompaction","summary":"compacted"}"#)
            ),
        ])

        XCTAssertEqual(messages.map(\.role), [.assistant])
        XCTAssertTrue(messages.filter { $0.role == .tool }.isEmpty)
        XCTAssertFalse(messages.contains { $0.text.localizedCaseInsensitiveContains("Filechange") })
        XCTAssertFalse(messages.contains { $0.text.localizedCaseInsensitiveContains("contextCompaction") })
    }

    func testDerivedToolTitlesPreserveCamelCaseToolNames() throws {
        XCTAssertEqual(GaryxMobileToolTraceEntry.title(for: "TaskCreate"), "TaskCreate")

        let transcriptMessages = [
            GaryxTranscriptMessage(
                index: 1,
                role: .toolUse,
                content: json(#"{"tool":"TaskCreate","input":{"title":"Review"}}"#)
            ),
            GaryxTranscriptMessage(
                index: 2,
                role: .toolUse,
                content: json(#"{"tool":"ToolSearch","input":{"query":"review"}}"#)
            ),
        ]
        let snapshot = GaryxRenderSnapshot(
            basedOnSeq: 3,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "turn:tools",
                    user: nil,
                    activity: [
                        .step(GaryxRenderStepRow(
                            id: "step:tools",
                            steps: [
                                .toolGroup(GaryxRenderToolGroup(
                                    id: "tool-group:1",
                                    status: .completed,
                                    entries: [
                                        GaryxRenderToolEntry(
                                            id: "entry:1",
                                            status: .completed,
                                            toolUse: GaryxRenderMessageRef(id: "seq:2", seq: 2, role: "tool_use")
                                        ),
                                        GaryxRenderToolEntry(
                                            id: "entry:2",
                                            status: .completed,
                                            toolUse: GaryxRenderMessageRef(id: "seq:3", seq: 3, role: "tool_use")
                                        ),
                                    ]
                                )),
                            ]
                        )),
                    ]
                )),
            ]
        )

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: [],
            transcriptMessages: transcriptMessages
        )
        guard case .turn(let turn) = try XCTUnwrap(rows.only?.activityRows.only) else {
            return XCTFail("expected server tool group turn")
        }
        guard case .toolGroup(let message) = try XCTUnwrap(turn.steps.only) else {
            return XCTFail("expected server tool group block")
        }
        let group = try XCTUnwrap(message.toolTraceGroup)
        XCTAssertEqual(group.entries.map(\.title), ["TaskCreate", "ToolSearch"])
        XCTAssertEqual(group.summary, "Used TaskCreate, ToolSearch")
    }

    private func json(_ raw: String) -> GaryxJSONValue {
        try! JSONDecoder().decode(GaryxJSONValue.self, from: Data(raw.utf8))
    }
}

private extension Array {
    var only: Element? {
        count == 1 ? self[0] : nil
    }
}
