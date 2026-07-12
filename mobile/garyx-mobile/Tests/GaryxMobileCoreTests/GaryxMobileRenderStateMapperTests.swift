@testable import GaryxMobileCore
import XCTest

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
            "filtered_placeholders": [
              {
                "message": { "id": "seq:3", "seq": 3, "role": "assistant" },
                "reason": "empty_streaming_assistant"
              }
            ],
            "window": {
              "floor_seq": 1,
              "has_more_above": true
            }
          }
        }
        """

        let frame = try JSONDecoder().decode(GaryxThreadRenderFrame.self, from: Data(json.utf8))

        XCTAssertEqual(frame.type, "thread_render_frame")
        XCTAssertEqual(frame.threadId, "thread::1")
        XCTAssertEqual(frame.events.only?.seq, 2)
        XCTAssertEqual(frame.events.only?.message?.id, "history:1")
        XCTAssertEqual(frame.renderState?.basedOnSeq, 2)
        XCTAssertEqual(frame.renderState?.tailActivity, .thinking)
        XCTAssertEqual(frame.renderState?.activeToolGroupId, "tool-group:active")
        XCTAssertEqual(frame.renderState?.progressLocus, .toolGroup)
        XCTAssertEqual(frame.renderState?.filteredPlaceholders.only?.reason, .emptyStreamingAssistant)
        XCTAssertEqual(frame.renderState?.window, GaryxRenderWindow(floorSeq: 1, hasMoreAbove: true))
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
            "filtered_placeholders": []
          }
        }
        """

        let frame = try JSONDecoder().decode(GaryxThreadRenderFrame.self, from: Data(json.utf8))

        XCTAssertEqual(frame.type, "thread_render_frame")
        XCTAssertEqual(frame.threadId, "thread::1")
        XCTAssertTrue(frame.events.isEmpty)
        XCTAssertEqual(frame.renderState?.basedOnSeq, 7)
        XCTAssertEqual(frame.renderState?.rows, [])
        XCTAssertEqual(frame.renderState?.tailActivity, GaryxRenderTailActivity.none)
    }

    func testCrossFloorTaskNotificationStaysAnOrdinaryServerOrderedRow() throws {
        let payload = #"""
        {
          "type": "thread_render_frame",
          "thread_id": "thread::render-floor-task-owner",
          "events": [
            {
              "type": "committed_message",
              "thread_id": "thread::render-floor-task-owner",
              "seq": 3,
              "message": {
                "index": 2,
                "role": "user",
                "text": "<garyx_task_notification event=\"ready_for_review\" task_id=\"#TASK-42\" status=\"in_review\">Test review</garyx_task_notification>"
              }
            },
            {
              "type": "committed_message",
              "thread_id": "thread::render-floor-task-owner",
              "seq": 4,
              "message": {
                "index": 3,
                "role": "tool_result",
                "tool_use_id": "tool-frame-boundary",
                "content": { "result": { "stdout": "done" } }
              }
            },
            {
              "type": "committed_message",
              "thread_id": "thread::render-floor-task-owner",
              "seq": 5,
              "message": {
                "index": 4,
                "role": "assistant",
                "text": "Notification handled"
              }
            }
          ],
          "render_state": {
            "based_on_seq": 5,
            "rows": [
              {
                "kind": "user_turn",
                "id": "user_turn:seq:3",
                "user": { "id": "seq:3", "seq": 3, "role": "user" },
                "activity": [
                  {
                    "kind": "assistant_reply",
                    "id": "assistant_reply:seq:5",
                    "message": { "id": "seq:5", "seq": 5, "role": "assistant" },
                    "streaming": false
                  }
                ],
                "started_at": null,
                "finished_at": null
              }
            ],
            "tailActivity": "none",
            "activeToolGroupId": null,
            "progress_locus": "none",
            "filtered_placeholders": [],
            "window": { "floor_seq": 3, "has_more_above": true }
          }
        }
        """#

        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 2, replayScope: .resume)
        let result = processor.processPayload(payload, threadId: "thread::render-floor-task-owner")
        XCTAssertNil(result.reconnect)
        XCTAssertEqual(result.actions.count, 2)
        guard case let .applyCommittedMessages(committed) = try XCTUnwrap(result.actions.first),
              case let .applyRenderSnapshot(snapshot) = try XCTUnwrap(result.actions.last) else {
            return XCTFail("frame must apply bodies before the server snapshot")
        }
        XCTAssertEqual(committed.map(\.index), [2, 3, 4])
        XCTAssertEqual(snapshot.window, GaryxRenderWindow(floorSeq: 3, hasMoreAbove: true))

        let messages = GaryxMobileTranscriptMapper.mobileMessages(from: committed)
        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: messages,
            transcriptMessages: committed
        )

        let row = try XCTUnwrap(rows.only)
        XCTAssertEqual(row.id, "user_turn:seq:3")
        XCTAssertTrue(row.userBlock?.message.text.contains("Test review") == true)
        guard case let .flat(reply) = try XCTUnwrap(row.activityRows.only) else {
            return XCTFail("the task notification should have one ordinary assistant reply")
        }
        XCTAssertEqual(reply.message.text, "Notification handled")
        XCTAssertEqual(
            rows.flatMap(\.activityRows).compactMap { activity -> GaryxMobileToolTraceGroup? in
                guard case let .turn(turn) = activity else { return nil }
                return turn.steps.compactMap(\.message.toolTraceGroup).first
            },
            [],
            "the cached late result body must not create local tool ownership"
        )
    }

    func testUserTurnDecodesMissingCapsuleCardsAsEmpty() throws {
        let row = try decodeRenderUserTurnRow(#"""
        {
          "kind": "user_turn",
          "id": "turn:missing-capsules",
          "user": { "id": "seq:1", "seq": 1, "role": "user" },
          "activity": [],
          "started_at": null,
          "finished_at": null
        }
        """#)

        XCTAssertTrue(row.capsuleCards.isEmpty)
    }

    func testCapsuleCardsDecodeAndMapAsRenderStatePassthrough() throws {
        let snapshot = try decodeRenderSnapshot(#"""
        {
          "based_on_seq": 2,
          "rows": [
            {
              "kind": "user_turn",
              "id": "turn:capsule",
              "user": { "id": "seq:1", "seq": 1, "role": "user" },
              "activity": [],
              "started_at": null,
              "finished_at": null,
              "capsule_cards": [
                {
                  "id": "capsule_card:01900000-0000-7000-8000-000000000901",
                  "capsule_id": "01900000-0000-7000-8000-000000000901",
                  "title": "Test Capsule",
                  "revision": 2,
                  "action": "updated"
                }
              ]
            }
          ],
          "tailActivity": "none",
          "activeToolGroupId": null,
          "progress_locus": "none",
          "filtered_placeholders": []
        }
        """#)

        guard case let .userTurn(renderRow) = try XCTUnwrap(snapshot.rows.only) else {
            return XCTFail("expected user_turn")
        }
        let renderCard = try XCTUnwrap(renderRow.capsuleCards.only)
        XCTAssertEqual(renderCard.capsuleId, "01900000-0000-7000-8000-000000000901")
        XCTAssertEqual(renderCard.title, "Test Capsule")
        XCTAssertEqual(renderCard.revision, 2)
        XCTAssertEqual(renderCard.action, .updated)

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: [mobileMessage(index: 0, role: .user, text: "Create a capsule")],
            transcriptMessages: []
        )

        let mappedCard = try XCTUnwrap(try XCTUnwrap(rows.only).capsuleCards.only)
        XCTAssertEqual(mappedCard, renderCard)
    }

    func testUnknownRenderKindsAreLossyAndDoNotDropSnapshot() throws {
        let snapshot = try decodeRenderSnapshot(#"""
        {
          "based_on_seq": 4,
          "rows": [
            {
              "kind": "future_row",
              "id": "future:row",
              "payload": { "ignored": true }
            },
            {
              "kind": "user_turn",
              "id": "turn:known",
              "user": { "id": "seq:1", "seq": 1, "role": "user" },
              "activity": [
                {
                  "kind": "future_activity",
                  "id": "future:activity",
                  "message": { "id": "seq:999", "seq": 999, "role": "assistant" }
                },
                {
                  "kind": "step",
                  "id": "step:known",
                  "steps": [
                    {
                      "kind": "future_step_item",
                      "id": "future:step",
                      "message": { "id": "seq:998", "seq": 998, "role": "assistant" }
                    },
                    {
                      "kind": "assistant_message",
                      "id": "assistant_step:seq:2",
                      "message": { "id": "seq:2", "seq": 2, "role": "assistant" },
                      "streaming": false
                    }
                  ],
                  "final_message": { "id": "seq:3", "seq": 3, "role": "assistant" },
                  "running": false,
                  "started_at": null,
                  "finished_at": null
                }
              ],
              "started_at": null,
              "finished_at": null
            }
          ],
          "tailActivity": "none",
          "activeToolGroupId": null,
          "progress_locus": "none",
          "filtered_placeholders": []
        }
        """#)

        XCTAssertEqual(snapshot.rows.count, 2)

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: [
                mobileMessage(index: 0, role: .user, text: "Question"),
                mobileMessage(index: 1, role: .assistant, text: "Intermediate"),
                mobileMessage(index: 2, role: .assistant, text: "Final"),
            ],
            transcriptMessages: []
        )

        let row = try XCTUnwrap(rows.only)
        guard case let .turn(turn) = try XCTUnwrap(row.activityRows.only) else {
            return XCTFail("known step should survive unknown siblings")
        }
        XCTAssertEqual(turn.steps.map(\.message.id), ["history:1"])
        XCTAssertEqual(turn.finalBlock?.message.id, "history:2")
    }

    func testCapsuleCardsDoNotCreateUnresolvedVisibleRefs() {
        let snapshot = GaryxRenderSnapshot(
            basedOnSeq: 1,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "turn:capsule-only",
                    user: ref(seq: 1, role: "user"),
                    activity: [],
                    capsuleCards: [
                        GaryxRenderCapsuleCard(
                            id: "capsule_card:01900000-0000-7000-8000-000000000902",
                            capsuleId: "01900000-0000-7000-8000-000000000902",
                            title: "Ignored for refs",
                            revision: 1,
                            action: .created
                        ),
                    ]
                )),
            ]
        )
        let cached = GaryxCachedTranscript(
            threadId: "thread::capsule-ref-test",
            savedAt: Date(timeIntervalSince1970: 0),
            messages: [
                GaryxTranscriptMessage(index: 0, role: .user, text: "Create capsule"),
            ],
            renderSnapshot: snapshot,
            hasMoreBefore: false,
            nextBeforeIndex: nil
        )

        let awaiting = GaryxSelectedThreadHistoryPresentation.isAwaitingInitialHistory(
            threadId: "thread::capsule-ref-test",
            historyLoaded: true,
            liveRenderSnapshot: nil,
            cachedTranscript: cached
        )

        XCTAssertFalse(awaiting)
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
        guard case let .flat(block) = try XCTUnwrap(row.activityRows.only) else {
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

        guard case let .turn(turn) = try XCTUnwrap(try XCTUnwrap(rows.only).activityRows.only) else {
            return XCTFail("step should map to an agent turn")
        }
        XCTAssertFalse(turn.isRunning)
        XCTAssertEqual(turn.finalBlock?.message.text, "Done")
        let toolBlock = try XCTUnwrap(turn.steps.only)
        guard case let .toolGroup(toolMessage) = toolBlock else {
            return XCTFail("expected tool group block")
        }
        XCTAssertEqual(toolMessage.text, "Ran 1 command")
        XCTAssertEqual(toolMessage.toolTraceGroup?.entries.only?.status, .completed)
        XCTAssertTrue(toolMessage.toolTraceGroup?.entries.only?.resultText?.contains("ok") == true)
    }

    func testCodexProjectionMapsOnlyCommandAndAggregatedOutput() throws {
        let command = #"/bin/zsh -lc "git status --short""#
        let output = " M README.md\n M Package.swift\n"
        let transcriptMessages = [
            GaryxTranscriptMessage(
                index: 1,
                role: .toolUse,
                content: json(#"""
                {
                  "type": "commandExecution",
                  "command": "/bin/zsh -lc \"git status --short\"",
                  "cwd": "/Users/test/repo",
                  "id": "exec-test"
                }
                """#),
                toolName: "commandExecution",
                toolUseId: "call-projection"
            ),
            GaryxTranscriptMessage(
                index: 2,
                role: .toolResult,
                content: json(#"""
                {
                  "type": "commandExecution",
                  "aggregatedOutput": " M README.md\n M Package.swift\n",
                  "cwd": "/Users/test/repo",
                  "id": "exec-test",
                  "status": "completed",
                  "exitCode": 0,
                  "durationMs": 12
                }
                """#),
                toolName: "commandExecution",
                toolUseId: "call-projection"
            ),
        ]
        let projection = GaryxRenderToolFieldProjection(
            toolName: "commandExecution",
            kind: .command,
            call: .init(
                root: .content,
                path: ["command"],
                format: .code,
                label: .command
            ),
            result: .init(
                root: .content,
                path: ["aggregatedOutput"],
                format: .code,
                label: .output
            ),
            status: "completed",
            exitCode: 0,
            durationMs: 12
        )
        let snapshot = GaryxRenderSnapshot(
            basedOnSeq: 3,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "turn:projection",
                    user: ref(seq: 1, role: "user"),
                    activity: [
                        .step(GaryxRenderStepRow(
                            id: "step:projection",
                            steps: [
                                .toolGroup(GaryxRenderToolGroup(
                                    id: "tool-group:projection",
                                    status: .completed,
                                    entries: [
                                        GaryxRenderToolEntry(
                                            id: "tool-entry:projection",
                                            toolUseId: "call-projection",
                                            status: .completed,
                                            toolUse: ref(seq: 2, role: "tool_use"),
                                            toolResult: ref(seq: 3, role: "tool_result"),
                                            projection: projection
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
            messages: [mobileMessage(index: 0, role: .user, text: "Inspect")],
            transcriptMessages: transcriptMessages
        )
        guard case let .turn(turn) = try XCTUnwrap(try XCTUnwrap(rows.only).activityRows.only),
              case let .toolGroup(toolMessage) = try XCTUnwrap(turn.steps.only) else {
            return XCTFail("expected projected tool group")
        }
        let entry = try XCTUnwrap(toolMessage.toolTraceGroup?.entries.only)
        XCTAssertEqual(entry.inputText, command)
        XCTAssertEqual(entry.inputLabel, "Command")
        XCTAssertEqual(entry.resultText, output)
        XCTAssertEqual(entry.resultLabel, "Output")
        XCTAssertEqual(entry.fieldProjection?.metadataText, "exit 0 · 12 ms")
        XCTAssertFalse(entry.resultText?.contains("/Users/test/repo") == true)
        XCTAssertFalse(entry.resultText?.contains("exec-test") == true)

        let detail = GaryxToolCallPresentation.detail(for: entry)
        XCTAssertEqual(detail.sections.map(\.label), ["Command", "Output"])
        XCTAssertEqual(detail.sections[0].content, .codeCard(command))
        XCTAssertEqual(detail.sections[1].content, .codeCard(output))
    }

    func testToolProjectionDecodesServerSnakeCaseFields() throws {
        let snapshot = try decodeRenderSnapshot(#"""
        {
          "based_on_seq": 2,
          "rows": [
            {
              "kind": "user_turn",
              "id": "turn:projection-wire",
              "user": null,
              "activity": [
                {
                  "kind": "step",
                  "id": "step:projection-wire",
                  "steps": [
                    {
                      "kind": "tool_group",
                      "id": "group:projection-wire",
                      "status": "completed",
                      "entries": [
                        {
                          "id": "entry:projection-wire",
                          "tool_use_id": "call-wire",
                          "status": "completed",
                          "tool_use": null,
                          "tool_result": null,
                          "projection": {
                            "tool_name": "commandExecution",
                            "kind": "command",
                            "visibility": "normal",
                            "call": {
                              "root": "content",
                              "format": "code",
                              "label": "command"
                            },
                            "exit_code": 0,
                            "duration_ms": 7
                          }
                        }
                      ],
                      "started_at": null,
                      "finished_at": null
                    }
                  ],
                  "final_message": null,
                  "running": false,
                  "started_at": null,
                  "finished_at": null
                }
              ],
              "started_at": null,
              "finished_at": null
            }
          ],
          "tailActivity": "none",
          "activeToolGroupId": null,
          "progress_locus": "none",
          "filtered_placeholders": []
        }
        """#)

        guard case let .userTurn(row) = try XCTUnwrap(snapshot.rows.only),
              case let .step(step) = try XCTUnwrap(row.activity.only),
              case let .toolGroup(group) = try XCTUnwrap(step.steps.only) else {
            return XCTFail("expected projected render entry")
        }
        let projection = try XCTUnwrap(group.entries.only?.projection)
        XCTAssertEqual(projection.kind, .command)
        XCTAssertEqual(projection.call?.path, [])
        XCTAssertEqual(projection.exitCode, 0)
        XCTAssertEqual(projection.durationMs, 7)
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

        guard case let .turn(turn) = try XCTUnwrap(try XCTUnwrap(rows.only).activityRows.only) else {
            return XCTFail("expected generic tool group turn")
        }
        XCTAssertTrue(turn.isRunning)
        guard case let .toolGroup(toolMessage) = try XCTUnwrap(turn.steps.only) else {
            return XCTFail("expected generic tool group")
        }
        XCTAssertTrue(toolMessage.isStreaming)
        let entry = try XCTUnwrap(toolMessage.toolTraceGroup?.entries.only)
        XCTAssertEqual(entry.title, "Tool")
        XCTAssertEqual(entry.status, .running)
    }

    func testOptimisticUserRowsAppendUntilOriginMaterializes() {
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
        guard case let .flat(newestAnswer) = try XCTUnwrap(rows[1].activityRows.only) else {
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
        guard case let .flat(answer) = try XCTUnwrap(row.activityRows.only) else {
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

    func testToolPayloadEnvelopeStillParsesButDoesNotDecideVisibility() {
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

    /// Timeline invariant for the "message flashes away when thinking appears"
    /// symptom family: drive the real frame processor with the first render
    /// frame of a new run (committed user body + thinking tail) and assert the
    /// just-sent user message stays visible, with a stable row identity, at
    /// every intermediate step of the send lifecycle.
    ///
    /// Steps mirror the app orchestration in GaryxMobileModel+ThreadStream:
    /// 0. optimistic local row appended (composer send)
    /// 1. frame arrives: bodies must be emitted before the render snapshot
    /// 2. bodies merged into the transcript cache, snapshot not yet swapped
    /// 3. new snapshot applied (visible `messages` flush still throttled)
    /// 4. throttled flush rebuilds visible messages via GaryxTranscriptMerge
    func testSendToThinkingFrameKeepsJustSentUserMessageVisibleAtEveryStep() throws {
        let earlierOrigin = "mobile-EARLIER0000000001"
        let origin = "mobile-00000000-0000-0000-0000-00000000000A"
        let sentText = "New question that mentions tool_use and mcp__ tools"
        let localRowId = "user_turn:origin:\(origin)"

        let priorTranscript = [
            GaryxTranscriptMessage(
                index: 0,
                role: .user,
                kind: "user_input",
                text: "Earlier question",
                metadata: json(#"{"origin_id":"\#(earlierOrigin)","client":"garyx-mobile"}"#)
            ),
            GaryxTranscriptMessage(
                index: 1,
                role: .assistant,
                kind: "assistant_reply",
                text: "Earlier answer"
            ),
        ]
        let idleSnapshot = GaryxRenderSnapshot(
            basedOnSeq: 2,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "user_turn:origin:\(earlierOrigin)",
                    user: ref(seq: 1, role: "user", id: "origin:\(earlierOrigin)"),
                    activity: [
                        .assistantReply(GaryxRenderAssistantReplyRow(
                            id: "reply:2",
                            message: ref(seq: 2, role: "assistant")
                        )),
                    ]
                )),
            ]
        )
        let priorVisibleMessages = GaryxMobileTranscriptMapper.mobileMessages(from: priorTranscript)
        let optimistic = GaryxMobileMessage(
            id: "origin:\(origin)",
            role: .user,
            text: sentText,
            timestamp: nil,
            isStreaming: false,
            clientIntentId: origin,
            localState: .optimistic
        )

        func assertSentMessageVisibleExactlyOnce(
            _ rows: [GaryxMobileTurnRow],
            expectedRowId: String,
            step: String
        ) throws {
            let matches = rows.filter { $0.userBlock?.message.text == sentText }
            XCTAssertEqual(matches.count, 1, "\(step): sent user message must be visible exactly once")
            let row = try XCTUnwrap(matches.first, step)
            XCTAssertEqual(row.id, expectedRowId, "\(step): row identity must stay stable (no remove+insert churn)")
            let user = try XCTUnwrap(row.userBlock?.message, step)
            XCTAssertNotEqual(
                GaryxMobileMessagePresentation.make(for: user),
                .historySkeleton,
                "\(step): sent user message must never degrade to the loading skeleton"
            )
        }

        // Step 0 — optimistic send: local row appended after server rows.
        let step0 = GaryxMobileRenderStateMapper.rows(
            snapshot: idleSnapshot,
            messages: priorVisibleMessages + [optimistic],
            transcriptMessages: priorTranscript
        )
        XCTAssertEqual(step0.map(\.id), ["user_turn:origin:\(earlierOrigin)", localRowId])
        try assertSentMessageVisibleExactlyOnce(step0, expectedRowId: localRowId, step: "step0")

        // Step 1 — first frame of the run arrives (committed user body +
        // thinking tail). The REST/stream projection may mark the user row
        // tool_related (nested-content sniffing); that must not matter.
        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 2, replayScope: .resume)
        let framePayload = """
        {
          "type": "thread_render_frame",
          "thread_id": "thread::timeline",
          "events": [
            {
              "type": "committed_message",
              "thread_id": "thread::timeline",
              "seq": 3,
              "message": {
                "index": 2,
                "role": "user",
                "kind": "user_input",
                "tool_related": true,
                "likely_user_visible": true,
                "text": "\(sentText)",
                "content": "\(sentText)",
                "metadata": { "origin_id": "\(origin)", "client": "garyx-mobile" }
              }
            }
          ],
          "render_state": {
            "based_on_seq": 3,
            "rows": [
              {
                "kind": "user_turn",
                "id": "user_turn:origin:\(earlierOrigin)",
                "user": { "id": "origin:\(earlierOrigin)", "seq": 1, "role": "user" },
                "activity": [
                  {
                    "kind": "assistant_reply",
                    "id": "reply:2",
                    "message": { "id": "seq:2", "seq": 2, "role": "assistant" },
                    "streaming": false
                  }
                ]
              },
              {
                "kind": "user_turn",
                "id": "user_turn:origin:\(origin)",
                "user": { "id": "origin:\(origin)", "seq": 3, "role": "user" },
                "activity": []
              }
            ],
            "tailActivity": "thinking",
            "progress_locus": "tail",
            "filtered_placeholders": []
          }
        }
        """
        let result = processor.processPayload(framePayload, threadId: "thread::timeline")
        XCTAssertNil(result.reconnect)
        XCTAssertEqual(result.actions.count, 2)
        guard case let .applyCommittedMessages(committed) = try XCTUnwrap(result.actions.first) else {
            return XCTFail("bodies must be applied before the render snapshot within one frame")
        }
        guard case let .applyRenderSnapshot(thinkingSnapshot) = try XCTUnwrap(result.actions.last) else {
            return XCTFail("the frame must end by applying the render snapshot")
        }
        XCTAssertEqual(thinkingSnapshot.tailActivity, .thinking)
        // The stream path rewrites committed identities to history:<seq-1>;
        // ref resolution must then work through the history index.
        XCTAssertEqual(committed.map(\.id), ["history:2"])
        XCTAssertEqual(committed.map(\.index), [2])

        // Step 2 — bodies merged into the transcript cache, snapshot not yet
        // swapped: the optimistic local row still carries the message.
        let transcriptAfterBodies = priorTranscript + committed
        let step2 = GaryxMobileRenderStateMapper.rows(
            snapshot: idleSnapshot,
            messages: priorVisibleMessages + [optimistic],
            transcriptMessages: transcriptAfterBodies
        )
        try assertSentMessageVisibleExactlyOnce(step2, expectedRowId: localRowId, step: "step2")

        // Step 3 — thinking snapshot applied while the visible messages array
        // has not flushed yet: the server row takes over the same identity.
        let step3 = GaryxMobileRenderStateMapper.rows(
            snapshot: thinkingSnapshot,
            messages: priorVisibleMessages + [optimistic],
            transcriptMessages: transcriptAfterBodies
        )
        XCTAssertEqual(step3.map(\.id), ["user_turn:origin:\(earlierOrigin)", localRowId])
        try assertSentMessageVisibleExactlyOnce(step3, expectedRowId: localRowId, step: "step3")

        // Step 4 — throttled flush rebuilds the visible projection through the
        // real merge; the committed body now backs the same row.
        let flushedMessages = GaryxTranscriptMerge.mergedMessages(
            GaryxMobileTranscriptMapper.mobileMessages(from: transcriptAfterBodies),
            withLocal: priorVisibleMessages + [optimistic]
        )
        let step4 = GaryxMobileRenderStateMapper.rows(
            snapshot: thinkingSnapshot,
            messages: flushedMessages,
            transcriptMessages: transcriptAfterBodies
        )
        try assertSentMessageVisibleExactlyOnce(step4, expectedRowId: localRowId, step: "step4")
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

    private func decodeRenderSnapshot(_ raw: String) throws -> GaryxRenderSnapshot {
        try JSONDecoder().decode(GaryxRenderSnapshot.self, from: Data(raw.utf8))
    }

    private func decodeRenderUserTurnRow(_ raw: String) throws -> GaryxRenderUserTurnRow {
        try JSONDecoder().decode(GaryxRenderUserTurnRow.self, from: Data(raw.utf8))
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
