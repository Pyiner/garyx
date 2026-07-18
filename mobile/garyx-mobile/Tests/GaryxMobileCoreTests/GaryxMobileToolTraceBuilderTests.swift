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

    /// Guardrail matrix for the TASK-1502 family: the ledger contract says a
    /// `role == user` transcript row is always a real user input (tool results
    /// ride dedicated tool roles), so no combination of polluted tool flags may
    /// ever make the transcript-to-mobile projection drop a user body. A
    /// dropped body starves `GaryxMobileRenderStateMapper` ref resolution and
    /// renders the gray history skeleton instead of the user's message.
    func testUserRowsAreNeverProjectedAwayAsToolTraces() {
        let hostileUserRows: [(label: String, row: GaryxTranscriptMessage)] = [
            (
                "kind=user_input + tool_related (captured TASK-1502 shape)",
                GaryxTranscriptMessage(
                    index: 10,
                    role: .user,
                    kind: "user_input",
                    text: "prose mentioning tool_use and mcp__ names",
                    toolRelated: true
                )
            ),
            (
                "hostile kind=tool_trace on a user row",
                GaryxTranscriptMessage(
                    index: 11,
                    role: .user,
                    kind: "tool_trace",
                    text: "user text under a hostile kind",
                    toolRelated: true
                )
            ),
            (
                "kind containing result substring",
                GaryxTranscriptMessage(
                    index: 12,
                    role: .user,
                    kind: "user_result_note",
                    text: "user text with a result-ish kind",
                    toolRelated: true
                )
            ),
            (
                "tool_name polluted on a user row",
                GaryxTranscriptMessage(
                    index: 13,
                    role: .user,
                    kind: "user_input",
                    text: "user text with tool_name set",
                    toolRelated: true,
                    toolName: "Bash"
                )
            ),
            (
                "tool_use_result flag polluted on a user row",
                GaryxTranscriptMessage(
                    index: 14,
                    role: .user,
                    kind: "user_input",
                    text: "user text with tool_use_result flag",
                    toolRelated: true,
                    toolUseResult: true
                )
            ),
            (
                "structured content with nested tool hints",
                GaryxTranscriptMessage(
                    index: 15,
                    role: .user,
                    kind: "user_input",
                    text: "structured user text",
                    content: json(#"[{"type":"text","text":"please call mcp__server__tool via tool_use"}]"#),
                    toolRelated: true
                )
            ),
        ]

        for (label, row) in hostileUserRows {
            XCTAssertNil(
                GaryxMobileTranscriptToolTraceClassifier.kind(for: row),
                "\(label): user rows must never classify as tool traces"
            )
            let projected = GaryxMobileTranscriptMapper.mobileMessages(from: [row])
            XCTAssertEqual(projected.count, 1, "\(label): user body must survive projection")
            XCTAssertEqual(projected.first?.role, .user, label)
            XCTAssertEqual(projected.first?.text, row.text, "\(label): user text must be preserved")
            XCTAssertEqual(projected.first?.historyIndex, row.index, label)
            if let message = projected.first {
                XCTAssertNotEqual(
                    GaryxMobileMessagePresentation.make(for: message),
                    .historySkeleton,
                    "\(label): projected user body must render as text, not skeleton"
                )
            }
        }
    }

    func testServerProjectedToolTitlesPreserveCamelCaseToolNames() throws {
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
                                            toolUse: GaryxRenderMessageRef(id: "seq:2", seq: 2, role: "tool_use"),
                                            projection: GaryxRenderToolFieldProjection(
                                                toolName: "TaskCreate",
                                                kind: .generic
                                            )
                                        ),
                                        GaryxRenderToolEntry(
                                            id: "entry:2",
                                            status: .completed,
                                            toolUse: GaryxRenderMessageRef(id: "seq:3", seq: 3, role: "tool_use"),
                                            projection: GaryxRenderToolFieldProjection(
                                                toolName: "ToolSearch",
                                                kind: .generic
                                            )
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

    func testMessageListSignatureChangesWhenProjectedDiffBodyArrivesLate() throws {
        let snapshot = GaryxRenderSnapshot(
            basedOnSeq: 1,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "turn:late-diff-body",
                    user: nil,
                    activity: [
                        .step(GaryxRenderStepRow(
                            id: "step:late-diff-body",
                            steps: [
                                .toolGroup(GaryxRenderToolGroup(
                                    id: "group:late-diff-body",
                                    status: .completed,
                                    entries: [
                                        GaryxRenderToolEntry(
                                            id: "entry:late-diff-body",
                                            status: .completed,
                                            toolUse: GaryxRenderMessageRef(
                                                id: "seq:1",
                                                seq: 1,
                                                role: "tool_use"
                                            ),
                                            projection: GaryxRenderToolFieldProjection(
                                                toolName: "Write",
                                                kind: .fileWrite,
                                                summary: GaryxRenderToolFieldSelector(
                                                    root: .content,
                                                    path: ["input", "file_path"],
                                                    format: .path,
                                                    label: .file
                                                ),
                                                diff: GaryxRenderToolDiffRecipe(
                                                    source: .toolUse,
                                                    segments: [
                                                        .pair(
                                                            old: nil,
                                                            new: GaryxRenderToolValueSelector(
                                                                root: .content,
                                                                path: ["input", "content"]
                                                            )
                                                        ),
                                                    ]
                                                )
                                            )
                                        ),
                                    ]
                                )),
                            ]
                        )),
                    ]
                )),
            ]
        )

        let beforeBody = try toolGroupMessage(snapshot: snapshot, transcriptMessages: [])
        let body = GaryxTranscriptMessage(
            index: 0,
            role: .toolUse,
            content: json(#"{"tool":"Write","input":{"file_path":"/Users/test/repo/Late.txt","content":"arrived"}}"#)
        )
        let afterBody = try toolGroupMessage(snapshot: snapshot, transcriptMessages: [body])

        XCTAssertEqual(beforeBody.toolTraceGroup?.entries.only?.fieldProjection?.diff, [])
        XCTAssertEqual(afterBody.toolTraceGroup?.entries.only?.fieldProjection?.diff.map(\.text), ["arrived"])
        XCTAssertNotEqual(
            GaryxMessageListSignature.make(for: [beforeBody]),
            GaryxMessageListSignature.make(for: [afterBody])
        )
    }

    private func toolGroupMessage(
        snapshot: GaryxRenderSnapshot,
        transcriptMessages: [GaryxTranscriptMessage]
    ) throws -> GaryxMobileMessage {
        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: [],
            transcriptMessages: transcriptMessages
        )
        guard case .turn(let turn) = try XCTUnwrap(rows.only?.activityRows.only),
              case .toolGroup(let message) = try XCTUnwrap(turn.steps.only) else {
            throw NSError(domain: "GaryxMobileToolTraceBuilderTests", code: 1)
        }
        return message
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
