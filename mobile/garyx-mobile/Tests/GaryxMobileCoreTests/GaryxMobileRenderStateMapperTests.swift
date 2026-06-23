import XCTest
@testable import GaryxMobileCore

final class GaryxMobileRenderStateMapperTests: XCTestCase {
    func testFrameDecodesServerFieldNames() throws {
        let json = """
        {
          "type": "thread_render_frame",
          "thread_id": "thread::1",
          "events": [
            {
              "type": "committed_message",
              "thread_id": "thread::1",
              "seq": 2,
              "message": { "index": 1, "role": "assistant", "text": "hello" }
            }
          ],
          "render_state": {
            "based_on_seq": 2,
            "rows": [
              {
                "kind": "user_turn",
                "id": "turn:1",
                "user": { "id": "seq:1", "seq": 1, "role": "user" },
                "activity": [
                  {
                    "kind": "assistant_reply",
                    "id": "reply:2",
                    "message": { "id": "seq:2", "seq": 2, "role": "assistant" },
                    "streaming": false
                  }
                ],
                "started_at": "2026-06-19T00:00:00Z",
                "finished_at": "2026-06-19T00:00:01Z"
              }
            ],
            "tailActivity": "thinking",
            "activeToolGroupId": "tool-group:active",
            "progress_locus": "tool_group",
            "visibleMessageIds": ["seq:1", "seq:2"],
            "filtered_placeholders": [
              {
                "message": { "id": "seq:3", "seq": 3, "role": "assistant" },
                "reason": "empty_streaming_assistant"
              }
            ]
          }
        }
        """

        let frame = try JSONDecoder().decode(GaryxThreadRenderFrame.self, from: Data(json.utf8))

        XCTAssertEqual(frame.type, "thread_render_frame")
        XCTAssertEqual(frame.threadId, "thread::1")
        XCTAssertEqual(frame.events.only?.seq, 2)
        XCTAssertEqual(frame.events.only?.message?.id, "history:1")
        XCTAssertEqual(frame.renderState.basedOnSeq, 2)
        XCTAssertEqual(frame.renderState.tailActivity, .thinking)
        XCTAssertEqual(frame.renderState.activeToolGroupId, "tool-group:active")
        XCTAssertEqual(frame.renderState.progressLocus, .toolGroup)
        XCTAssertEqual(frame.renderState.visibleMessageIds, ["seq:1", "seq:2"])
        XCTAssertEqual(frame.renderState.filteredPlaceholders.only?.reason, .emptyStreamingAssistant)
    }

    func testFrameDecodesSnapshotOnlyInitialFrame() throws {
        let json = """
        {
          "type": "thread_render_frame",
          "thread_id": "thread::1",
          "events": [],
          "render_state": {
            "based_on_seq": 7,
            "rows": [],
            "tailActivity": "none",
            "activeToolGroupId": null,
            "progress_locus": "none",
            "visibleMessageIds": [],
            "filtered_placeholders": []
          }
        }
        """

        let frame = try JSONDecoder().decode(GaryxThreadRenderFrame.self, from: Data(json.utf8))

        XCTAssertEqual(frame.type, "thread_render_frame")
        XCTAssertEqual(frame.threadId, "thread::1")
        XCTAssertTrue(frame.events.isEmpty)
        XCTAssertEqual(frame.renderState.basedOnSeq, 7)
        XCTAssertEqual(frame.renderState.rows, [])
        XCTAssertEqual(frame.renderState.tailActivity, .none)
    }

    func testMapsAssistantReplyFromServerRowsUsingSeqPrimaryRefs() throws {
        let messages = [
            mobileMessage(index: 0, role: .user, text: "Question", id: "local-user"),
            mobileMessage(index: 1, role: .assistant, text: "Answer", id: "history:1"),
        ]
        let snapshot = GaryxRenderSnapshot(
            basedOnSeq: 2,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "turn:seq1",
                    user: ref(seq: 1, role: "user"),
                    activity: [
                        .assistantReply(GaryxRenderAssistantReplyRow(
                            id: "reply:seq2",
                            message: ref(seq: 2, role: "assistant")
                        )),
                    ]
                )),
            ]
        )

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: messages,
            transcriptMessages: []
        )

        let row = try XCTUnwrap(rows.only)
        XCTAssertEqual(row.id, "turn:seq1")
        XCTAssertEqual(row.userBlock?.message.id, "local-user")
        guard case .flat(let block) = try XCTUnwrap(row.activityRows.only) else {
            return XCTFail("assistant_reply should map to a flat block")
        }
        XCTAssertEqual(block.message.text, "Answer")
    }

    func testMapsToolGroupFromServerRowsWithoutLocalGrouping() throws {
        let transcriptMessages = [
            toolUse(index: 1, toolUseId: "call-1", command: "ls"),
            toolResult(index: 2, toolUseId: "call-1", output: "ok"),
        ]
        let messages = [
            mobileMessage(index: 0, role: .user, text: "List files"),
            mobileMessage(index: 3, role: .assistant, text: "Done"),
        ]
        let snapshot = GaryxRenderSnapshot(
            basedOnSeq: 4,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "turn:seq1",
                    user: ref(seq: 1, role: "user"),
                    activity: [
                        .step(GaryxRenderStepRow(
                            id: "step:tools",
                            steps: [
                                .toolGroup(GaryxRenderToolGroup(
                                    id: "tool-group:1",
                                    status: .completed,
                                    entries: [
                                        GaryxRenderToolEntry(
                                            id: "tool-entry:1",
                                            toolUseId: "call-1",
                                            status: .completed,
                                            toolUse: ref(seq: 2, role: "tool_use"),
                                            toolResult: ref(seq: 3, role: "tool_result")
                                        ),
                                    ]
                                )),
                            ],
                            finalMessage: ref(seq: 4, role: "assistant")
                        )),
                    ]
                )),
            ]
        )

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: messages,
            transcriptMessages: transcriptMessages
        )

        guard case .turn(let turn) = try XCTUnwrap(try XCTUnwrap(rows.only).activityRows.only) else {
            return XCTFail("step should map to an agent turn")
        }
        XCTAssertFalse(turn.isRunning)
        XCTAssertEqual(turn.finalBlock?.message.text, "Done")
        let toolBlock = try XCTUnwrap(turn.steps.only)
        guard case .toolGroup(let toolMessage) = toolBlock else {
            return XCTFail("expected tool group block")
        }
        XCTAssertEqual(toolMessage.text, "Ran 1 command")
        XCTAssertEqual(toolMessage.toolTraceGroup?.entries.only?.status, .completed)
        XCTAssertTrue(toolMessage.toolTraceGroup?.entries.only?.resultText?.contains("ok") == true)
    }

    func testToolEntryFallsBackToGenericWhenRefsAreMissing() throws {
        let snapshot = GaryxRenderSnapshot(
            basedOnSeq: 1,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "turn:tool-only",
                    user: nil,
                    activity: [
                        .step(GaryxRenderStepRow(
                            id: "step:generic",
                            steps: [
                                .toolGroup(GaryxRenderToolGroup(
                                    id: "tool-group:generic",
                                    status: .active,
                                    entries: [
                                        GaryxRenderToolEntry(id: "tool-entry:generic", status: .running),
                                    ]
                                )),
                            ],
                            running: true
                        )),
                    ]
                )),
            ],
            tailActivity: .toolActive,
            activeToolGroupId: "tool-group:generic",
            progressLocus: .toolGroup
        )

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: [],
            transcriptMessages: []
        )

        guard case .turn(let turn) = try XCTUnwrap(try XCTUnwrap(rows.only).activityRows.only) else {
            return XCTFail("expected generic tool group turn")
        }
        XCTAssertTrue(turn.isRunning)
        guard case .toolGroup(let toolMessage) = try XCTUnwrap(turn.steps.only) else {
            return XCTFail("expected generic tool group")
        }
        XCTAssertTrue(toolMessage.isStreaming)
        let entry = try XCTUnwrap(toolMessage.toolTraceGroup?.entries.only)
        XCTAssertEqual(entry.title, "Tool")
        XCTAssertEqual(entry.status, .running)
    }

    func testOptimisticUserRowsAppendUntilOriginMaterializes() throws {
        let materializedOrigin = "00000000-0000-0000-0000-000000000001"
        let pendingOrigin = "00000000-0000-0000-0000-000000000002"
        var materialized = mobileMessage(
            index: 0,
            role: .user,
            text: "Sent",
            id: "origin:\(materializedOrigin)"
        )
        materialized.localState = .optimistic
        let pending = GaryxMobileMessage(
            id: "origin:\(pendingOrigin)",
            role: .user,
            text: "Still pending",
            timestamp: nil,
            isStreaming: false,
            clientIntentId: pendingOrigin,
            localState: .optimistic
        )
        let snapshot = GaryxRenderSnapshot(
            basedOnSeq: 1,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "user_turn:origin:\(materializedOrigin)",
                    user: ref(seq: 1, role: "user", id: "origin:\(materializedOrigin)"),
                    activity: []
                )),
            ]
        )

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: [materialized, pending],
            transcriptMessages: []
        )

        XCTAssertEqual(
            rows.map(\.id),
            [
                "user_turn:origin:\(materializedOrigin)",
                "user_turn:origin:\(pendingOrigin)",
            ]
        )
        XCTAssertEqual(rows.first?.userBlock?.message.id, "origin:\(materializedOrigin)")
        XCTAssertEqual(rows.last?.userBlock?.message.text, "Still pending")
    }

    func testMissingUserRefMapsToLoadingUserPlaceholder() throws {
        let snapshot = GaryxRenderSnapshot(
            basedOnSeq: 99,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "turn:missing",
                    user: ref(seq: 99, role: "user"),
                    activity: []
                )),
            ]
        )

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: [mobileMessage(index: 0, role: .user, text: "Unreferenced")],
            transcriptMessages: []
        )

        let row = try XCTUnwrap(rows.only)
        let user = try XCTUnwrap(row.userBlock?.message)
        XCTAssertEqual(user.id, "history:98")
        XCTAssertEqual(user.historyIndex, 98)
        XCTAssertEqual(user.role, .user)
        XCTAssertEqual(user.text, "")
        XCTAssertTrue(user.isStreaming)
        XCTAssertEqual(user.localState, .remotePartial)
    }

    func testFullSnapshotWithOnlyNewestBodiesLeavesOlderTurnAsLoadingPlaceholder() throws {
        let snapshot = GaryxRenderSnapshot(
            basedOnSeq: 4,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "turn:old",
                    user: ref(seq: 1, role: "user"),
                    activity: [
                        .assistantReply(GaryxRenderAssistantReplyRow(
                            id: "reply:old",
                            message: ref(seq: 2, role: "assistant")
                        )),
                    ]
                )),
                .userTurn(GaryxRenderUserTurnRow(
                    id: "turn:new",
                    user: ref(seq: 3, role: "user"),
                    activity: [
                        .assistantReply(GaryxRenderAssistantReplyRow(
                            id: "reply:new",
                            message: ref(seq: 4, role: "assistant")
                        )),
                    ]
                )),
            ]
        )
        let messages = [
            mobileMessage(index: 2, role: .user, text: "Newest question"),
            mobileMessage(index: 3, role: .assistant, text: "Newest answer"),
        ]

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: messages,
            transcriptMessages: []
        )

        XCTAssertEqual(rows.map(\.id), ["turn:old", "turn:new"])
        XCTAssertEqual(rows[0].userBlock?.message.id, "history:0")
        XCTAssertEqual(rows[0].userBlock?.message.localState, .remotePartial)
        XCTAssertTrue(rows[0].activityRows.isEmpty)
        XCTAssertEqual(rows[1].userBlock?.message.text, "Newest question")
        guard case .flat(let newestAnswer) = try XCTUnwrap(rows[1].activityRows.only) else {
            return XCTFail("newest assistant reply should resolve from the one-turn cache")
        }
        XCTAssertEqual(newestAnswer.message.text, "Newest answer")
    }

    func testWindowedSnapshotWithOneTurnBodiesMapsSingleCompleteTurn() throws {
        let snapshot = GaryxRenderSnapshot(
            basedOnSeq: 4,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "turn:new",
                    user: ref(seq: 3, role: "user"),
                    activity: [
                        .assistantReply(GaryxRenderAssistantReplyRow(
                            id: "reply:new",
                            message: ref(seq: 4, role: "assistant")
                        )),
                    ]
                )),
            ]
        )
        let messages = [
            mobileMessage(index: 2, role: .user, text: "Newest question"),
            mobileMessage(index: 3, role: .assistant, text: "Newest answer"),
        ]

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: messages,
            transcriptMessages: []
        )

        let row = try XCTUnwrap(rows.only)
        XCTAssertEqual(row.id, "turn:new")
        XCTAssertEqual(row.userBlock?.message.text, "Newest question")
        guard case .flat(let answer) = try XCTUnwrap(row.activityRows.only) else {
            return XCTFail("assistant reply should map to a flat block")
        }
        XCTAssertEqual(answer.message.text, "Newest answer")
    }

    func testNilSnapshotRendersOptimisticUserRows() {
        let committed = mobileMessage(index: 0, role: .assistant, text: "Cached")
        let pendingOrigin = "00000000-0000-0000-0000-000000000003"
        let pending = GaryxMobileMessage(
            id: "origin:\(pendingOrigin)",
            role: .user,
            text: "Pending",
            timestamp: nil,
            isStreaming: false,
            clientIntentId: pendingOrigin,
            localState: .optimistic
        )

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: nil,
            messages: [committed, pending],
            transcriptMessages: []
        )

        XCTAssertEqual(rows.map(\.id), ["user_turn:origin:\(pendingOrigin)"])
        XCTAssertEqual(rows.only?.userBlock?.message.text, "Pending")
    }

    func testFailedLocalUserRowRendersRetryEntryWithoutSnapshot() {
        let failedOrigin = "00000000-0000-0000-0000-000000000004"
        var failed = GaryxMobileMessage(
            id: "origin:\(failedOrigin)",
            role: .user,
            text: "Retry me",
            timestamp: nil,
            isStreaming: false,
            clientIntentId: failedOrigin,
            localState: .optimistic
        )
        failed.statusText = "Gateway unavailable"

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: nil,
            messages: [failed],
            transcriptMessages: []
        )

        XCTAssertEqual(rows.map(\.id), ["user_turn:origin:\(failedOrigin)"])
        XCTAssertEqual(rows.only?.userBlock?.message.statusText, "Gateway unavailable")
    }

    func testCommittedUserMessageUsesOriginIdFromMetadata() {
        let originId = "00000000-0000-0000-0000-000000000001"
        let message = GaryxTranscriptMessage(
            index: 1,
            role: .user,
            text: "Hello",
            metadata: json(#"{"origin_id":"\#(originId)"}"#)
        )

        let mobileMessages = GaryxMobileTranscriptMapper.mobileMessages(from: [message])

        XCTAssertEqual(message.id, "origin:\(originId)")
        XCTAssertEqual(message.originId, originId)
        XCTAssertEqual(mobileMessages.only?.id, "origin:\(originId)")
        XCTAssertEqual(mobileMessages.only?.clientIntentId, originId)
    }

    func testToolPayloadEnvelopeStillParsesButDoesNotDecideVisibility() throws {
        let child = GaryxTranscriptMessage(
            index: 1,
            role: .toolUse,
            content: json(#"{"input":{"command":"ls"},"tool":"Bash"}"#),
            metadata: json(#"{"parent_tool_use_id":"toolu_PARENT","source":"claude_sdk"}"#)
        )
        let payload = GaryxMobileToolTracePayload.fromTranscript(child)

        XCTAssertEqual(child.garyxParentToolUseId, "toolu_PARENT")
        XCTAssertEqual(payload.parentToolUseId, "toolu_PARENT")
        XCTAssertEqual(payload.normalizedToolName, "Bash")
        XCTAssertEqual(payload.summaryText, "ls")
    }

    private func mobileMessage(
        index: Int,
        role: GaryxMobileMessage.Role,
        text: String,
        id: String? = nil
    ) -> GaryxMobileMessage {
        GaryxMobileMessage(
            id: id ?? "history:\(index)",
            role: role,
            text: text,
            timestamp: nil,
            isStreaming: false,
            localState: .remoteFinal,
            historyIndex: index
        )
    }

    private func ref(seq: Int, role: String, id: String? = nil) -> GaryxRenderMessageRef {
        GaryxRenderMessageRef(id: id ?? "seq:\(seq)", seq: seq, role: role)
    }

    private func toolUse(index: Int, toolUseId: String, command: String) -> GaryxTranscriptMessage {
        GaryxTranscriptMessage(
            index: index,
            role: .toolUse,
            content: json(#"{"tool":"Bash","input":{"command":"\#(command)"}}"#),
            toolUseId: toolUseId
        )
    }

    private func toolResult(index: Int, toolUseId: String, output: String) -> GaryxTranscriptMessage {
        GaryxTranscriptMessage(
            index: index,
            role: .toolResult,
            content: json(#"{"result":{"stdout":"\#(output)"}}"#),
            toolUseId: toolUseId
        )
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
