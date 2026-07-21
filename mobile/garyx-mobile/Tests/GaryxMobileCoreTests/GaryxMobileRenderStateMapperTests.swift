@testable import GaryxMobileCore
import XCTest

final class GaryxMobileRenderStateMapperTests: XCTestCase {
    /// Passing boundary check for the #TASK-2511 RED gateway reproduction.
    ///
    /// This is the sanitized wire shape captured from the affected thread:
    /// the committed user body carries an image backed by a managed `payload`
    /// path, while render_state references that body by seq/origin id. It pins
    /// that SSE decoding and the dumb render-state mapper do not drop the
    /// attachment before the thumbnail loader receives it.
    func testCapturedManagedImageFramePreservesAttachmentThroughRenderMapper() throws {
        let path = "/Users/test/.garyx/data/prompt-attachments-v1/attachment:00000000-0000-0000-0000-000000000001/payload"
        let raw = #"""
        {
          "type": "thread_render_frame",
          "thread_id": "thread::image-echo-repro",
          "events": [
            {
              "type": "committed_message",
              "thread_id": "thread::image-echo-repro",
              "seq": 1,
              "message": {
                "role": "system",
                "kind": "control",
                "internal": true,
                "control": { "kind": "run_start" }
              }
            },
            {
              "type": "committed_message",
              "thread_id": "thread::image-echo-repro",
              "seq": 2,
              "message": {
                "role": "user",
                "text": "Test image prompt",
                "content": [
                  { "type": "text", "text": "Test image prompt" },
                  {
                    "type": "image",
                    "media_type": "image/jpeg",
                    "name": "photo-1.jpg",
                    "path": "\#(path)"
                  }
                ],
                "metadata": {
                  "origin_id": "mobile-00000000-0000-0000-0000-000000000001",
                  "client": "garyx-mobile",
                  "attachments": [
                    {
                      "attachment_id": "attachment:00000000-0000-0000-0000-000000000001",
                      "kind": "image",
                      "media_type": "image/jpeg",
                      "name": "photo-1.jpg",
                      "path": "\#(path)"
                    }
                  ]
                }
              }
            }
          ],
          "render_state": {
            "based_on_seq": 2,
            "rows": [
              {
                "kind": "user_turn",
                "id": "user_turn:origin:mobile-00000000-0000-0000-0000-000000000001",
                "user": {
                  "id": "origin:mobile-00000000-0000-0000-0000-000000000001",
                  "seq": 2,
                  "role": "user"
                },
                "activity": [],
                "started_at": null,
                "finished_at": null
              }
            ],
            "tailActivity": "none",
            "activeToolGroupId": null,
            "progress_locus": "none",
            "filtered_placeholders": []
          }
        }
        """#

        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 0, replayScope: .resume)
        let result = processor.processPayload(raw, threadId: "thread::image-echo-repro")
        XCTAssertNil(result.reconnect)
        guard case let .applyCommittedMessages(committed) = try XCTUnwrap(result.actions.first),
              case let .applyRenderSnapshot(snapshot) = try XCTUnwrap(result.actions.last) else {
            return XCTFail("frame must apply committed bodies before render_state")
        }

        let messages = GaryxMobileTranscriptMapper.mobileMessages(from: committed)
        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: messages,
            transcriptMessages: committed
        )
        let attachment = try XCTUnwrap(rows.only?.userBlock?.message.attachments.only)

        XCTAssertEqual(attachment.name, "photo-1.jpg")
        XCTAssertEqual(attachment.mediaType, "image/jpeg")
        XCTAssertEqual(attachment.path, path)
        XCTAssertNil(attachment.dataUrl)
        XCTAssertNil(attachment.remoteUrl)
        XCTAssertEqual(
            GaryxMobileFileLink.previewTarget(
                forLocalFilePath: try XCTUnwrap(attachment.path),
                workspacePaths: []
            ),
            GaryxMobileWorkspaceFileTarget(
                workspaceDir: "/Users/test/.garyx/data/prompt-attachments-v1/attachment:00000000-0000-0000-0000-000000000001",
                path: "payload"
            )
        )
    }

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
                "user": {
                  "id": "seq:3",
                  "seq": 3,
                  "role": "user",
                  "presentation": {
                    "kind": "task_notification",
                    "event": "ready_for_review",
                    "status": "in_review",
                    "task_id": "#TASK-42",
                    "title": "Test review"
                  }
                },
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
        XCTAssertNil(
            messages.first(where: { $0.historyIndex == 2 })?.renderPresentation,
            "body mapping alone must not infer presentation from XML"
        )
        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: messages,
            transcriptMessages: committed
        )

        let row = try XCTUnwrap(rows.only)
        XCTAssertEqual(row.id, "user_turn:seq:3")
        XCTAssertTrue(row.userBlock?.message.text.contains("Test review") == true)
        XCTAssertEqual(
            row.userBlock?.message.renderPresentation,
            .taskNotification(
                event: "ready_for_review",
                status: "in_review",
                taskId: "#TASK-42",
                title: "Test review"
            )
        )
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

    func testUnknownPresentationKindDegradesOnlyTheMessageHint() throws {
        let row = try decodeRenderUserTurnRow(#"""
        {
          "kind": "user_turn",
          "id": "turn:future-presentation",
          "user": {
            "id": "seq:1",
            "seq": 1,
            "role": "user",
            "presentation": {
              "kind": "future_card",
              "event": "future_event",
              "status": "future_status",
              "task_id": "#TASK-1",
              "title": "Future title"
            }
          },
          "activity": [],
          "started_at": null,
          "finished_at": null
        }
        """#)

        let ref = try XCTUnwrap(row.user)
        XCTAssertEqual(ref.id, "seq:1")
        XCTAssertNil(ref.presentation)

        let mapped = GaryxMobileRenderStateMapper.rows(
            snapshot: GaryxRenderSnapshot(
                basedOnSeq: 1,
                rows: [.userTurn(row)],
                tailActivity: .none,
                activeToolGroupId: nil,
                progressLocus: .none,
                filteredPlaceholders: []
            ),
            messages: [mobileMessage(index: 0, role: .user, text: "ordinary body")],
            transcriptMessages: []
        )
        XCTAssertNil(mapped.only?.userBlock?.message.renderPresentation)
    }

    func testFrozenOldIOSFullSnapshotDropsRowWithObjectPresentation() throws {
        let frame = try JSONDecoder().decode(
            FrozenStringPresentationFrame.self,
            from: Data(#"""
            {
              "type": "thread_render_frame",
              "thread_id": "thread::old-ios-full",
              "render_state": {
                "based_on_seq": 2,
                "rows": [
                  {
                    "kind": "user_turn",
                    "id": "turn:task-notification",
                    "user": {
                      "id": "seq:1",
                      "seq": 1,
                      "role": "user",
                      "presentation": {
                        "kind": "task_notification",
                        "event": "ready_for_review",
                        "status": "in_review",
                        "task_id": "#TASK-42",
                        "title": "Review fixture"
                      }
                    }
                  },
                  {
                    "kind": "user_turn",
                    "id": "turn:ordinary",
                    "user": { "id": "seq:2", "seq": 2, "role": "user" }
                  }
                ]
              }
            }
            """#.utf8)
        )

        XCTAssertEqual(frame.renderState?.rows.map(\.id), ["turn:ordinary"])
    }

    func testFrozenOldIOSDeltaWithObjectPresentationTakesGapReplayPath() throws {
        let frame = try JSONDecoder().decode(
            FrozenStringPresentationFrame.self,
            from: Data(#"""
            {
              "type": "thread_render_frame",
              "thread_id": "thread::old-ios-delta",
              "render_delta": {
                "from_seq": 1,
                "from_rows_hash": "11",
                "based_on_seq": 2,
                "rows_hash": "22",
                "row_order": ["turn:task-notification"],
                "upsert_rows": [
                  {
                    "kind": "user_turn",
                    "id": "turn:task-notification",
                    "user": {
                      "id": "seq:2",
                      "seq": 2,
                      "role": "user",
                      "presentation": {
                        "kind": "task_notification",
                        "event": "ready_for_review",
                        "status": "in_review",
                        "task_id": "#TASK-42",
                        "title": "Review fixture"
                      }
                    }
                  }
                ]
              }
            }
            """#.utf8)
        )

        XCTAssertEqual(frame.deltaResolution, .gapReplay)
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
                                            toolResult: ref(seq: 3, role: "tool_result"),
                                            projection: GaryxRenderToolFieldProjection(
                                                toolName: "Bash",
                                                kind: .command,
                                                call: .init(
                                                    root: .content,
                                                    path: ["input", "command"],
                                                    format: .code,
                                                    label: .command
                                                ),
                                                result: .init(
                                                    root: .content,
                                                    path: ["result", "stdout"],
                                                    format: .code,
                                                    label: .output
                                                )
                                            )
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

    func testClaudeCommandKeepsDescriptionInRowAndCommandInDetail() throws {
        let description = "Read schema definition"
        let command = "sed -n '5,60p' src/schema.rs"
        let entries = try mappedToolEntries(fromFrame: #"""
        {
          "type": "thread_render_frame",
          "thread_id": "thread::command-detail",
          "events": [
            {
              "type": "committed_message",
              "thread_id": "thread::command-detail",
              "seq": 1,
              "message": {
                "index": 0,
                "role": "tool_use",
                "tool_name": "Bash",
                "tool_use_id": "call-command-detail",
                "content": {
                  "tool": "Bash",
                  "input": {
                    "description": "Read schema definition",
                    "command": "sed -n '5,60p' src/schema.rs"
                  }
                }
              }
            }
          ],
          "render_state": {
            "based_on_seq": 1,
            "rows": [
              {
                "kind": "user_turn",
                "id": "turn:command-detail",
                "user": null,
                "activity": [
                  {
                    "kind": "step",
                    "id": "step:command-detail",
                    "steps": [
                      {
                        "kind": "tool_group",
                        "id": "group:command-detail",
                        "status": "completed",
                        "entries": [
                          {
                            "id": "entry:command-detail",
                            "tool_use_id": "call-command-detail",
                            "status": "completed",
                            "tool_use": { "id": "seq:1", "seq": 1, "role": "tool_use" },
                            "tool_result": null,
                            "projection": {
                              "tool_name": "Bash",
                              "kind": "command",
                              "visibility": "normal",
                              "summary": {
                                "root": "content",
                                "path": ["input", "description"],
                                "format": "text",
                                "label": "call"
                              },
                              "call": {
                                "root": "content",
                                "path": ["input", "command"],
                                "format": "code",
                                "label": "command"
                              }
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
        }
        """#)

        let entry = try XCTUnwrap(entries.only)
        XCTAssertEqual(entry.summaryText, description)
        XCTAssertEqual(entry.inputText, command)

        let detail = GaryxToolCallPresentation.detail(for: entry)
        XCTAssertEqual(detail.sections.map(\.label), ["Command"])
        XCTAssertEqual(detail.sections.only?.content, .codeCard(command))
    }

    func testStructuredWebParametersStayOutOfCollapsedRowSummary() throws {
        let entries = try mappedToolEntries(fromFrame: #"""
        {
          "type": "thread_render_frame",
          "thread_id": "thread::web-parameters",
          "events": [
            {
              "type": "committed_message",
              "thread_id": "thread::web-parameters",
              "seq": 1,
              "message": {
                "index": 0,
                "role": "tool_use",
                "tool_name": "webSearch",
                "tool_use_id": "exec-00000000-0000-0000-0000-000000000001",
                "content": {
                  "action": null,
                  "id": "exec-00000000-0000-0000-0000-000000000001",
                  "query": "",
                  "type": "webSearch"
                }
              }
            }
          ],
          "render_state": {
            "based_on_seq": 1,
            "rows": [
              {
                "kind": "user_turn",
                "id": "turn:web-parameters",
                "user": null,
                "activity": [
                  {
                    "kind": "step",
                    "id": "step:web-parameters",
                    "steps": [
                      {
                        "kind": "tool_group",
                        "id": "group:web-parameters",
                        "status": "completed",
                        "entries": [
                          {
                            "id": "entry:web-parameters",
                            "tool_use_id": "exec-00000000-0000-0000-0000-000000000001",
                            "status": "completed",
                            "tool_use": { "id": "seq:1", "seq": 1, "role": "tool_use" },
                            "tool_result": null,
                            "projection": {
                              "tool_name": "webSearch",
                              "kind": "web",
                              "visibility": "normal",
                              "call": {
                                "root": "content",
                                "format": "json",
                                "label": "parameters"
                              }
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
        }
        """#)

        let entry = try XCTUnwrap(entries.only)
        XCTAssertNil(entry.summaryText)
        XCTAssertNil(GaryxToolCallPresentation.listRows(from: [entry]).only?.detail)

        let inputText = try XCTUnwrap(entry.inputText)
        XCTAssertTrue(inputText.contains("webSearch"))
        let detail = GaryxToolCallPresentation.detail(for: entry)
        XCTAssertEqual(detail.sections.map(\.label), ["Parameters"])
        XCTAssertEqual(detail.sections.only?.content, .codeCard(inputText))
    }

    func testNativeImageGenerationResolvesResultOwnedPromptAndSavedPath() throws {
        let prompt = "A synthetic lighthouse beneath a violet evening sky."
        let imagePath = "/Users/test/.codex/generated_images/synthetic/exec-native.png"
        let transcriptMessages = [
            GaryxTranscriptMessage(
                index: 1,
                role: .toolUse,
                content: json(#"""
                {
                  "id": "exec-native",
                  "result": "",
                  "revisedPrompt": null,
                  "status": "in_progress",
                  "type": "imageGeneration"
                }
                """#),
                toolName: "imageGeneration",
                toolUseId: "call-image-generation"
            ),
            GaryxTranscriptMessage(
                index: 2,
                role: .toolResult,
                content: json(#"""
                {
                  "id": "exec-native",
                  "result": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
                  "revisedPrompt": "A synthetic lighthouse beneath a violet evening sky.",
                  "savedPath": "/Users/test/.codex/generated_images/synthetic/exec-native.png",
                  "status": "completed",
                  "type": "imageGeneration"
                }
                """#),
                toolName: "imageGeneration",
                toolUseId: "call-image-generation"
            ),
        ]
        let projection = GaryxRenderToolFieldProjection(
            toolName: "imageGeneration",
            kind: .image,
            call: .init(
                root: .content,
                path: ["revisedPrompt"],
                format: .text,
                label: .prompt
            ),
            result: .init(
                root: .content,
                path: ["savedPath"],
                format: .image,
                label: .image
            ),
            status: "completed"
        )
        let snapshot = GaryxRenderSnapshot(
            basedOnSeq: 3,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "turn:image-generation",
                    user: ref(seq: 1, role: "user"),
                    activity: [
                        .step(GaryxRenderStepRow(
                            id: "step:image-generation",
                            steps: [
                                .toolGroup(GaryxRenderToolGroup(
                                    id: "tool-group:image-generation",
                                    status: .completed,
                                    entries: [
                                        GaryxRenderToolEntry(
                                            id: "tool-entry:image-generation",
                                            toolUseId: "call-image-generation",
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
            messages: [mobileMessage(index: 0, role: .user, text: "Generate")],
            transcriptMessages: transcriptMessages
        )
        guard case let .turn(turn) = try XCTUnwrap(try XCTUnwrap(rows.only).activityRows.only),
              case let .toolGroup(toolMessage) = try XCTUnwrap(turn.steps.only) else {
            return XCTFail("expected projected image-generation tool group")
        }
        let entry = try XCTUnwrap(toolMessage.toolTraceGroup?.entries.only)

        XCTAssertEqual(entry.title, "Image")
        XCTAssertEqual(entry.inputText, prompt)
        XCTAssertEqual(entry.inputLabel, "Prompt")
        XCTAssertEqual(entry.resultText, imagePath)
        XCTAssertEqual(entry.resultLabel, "Image")
        XCTAssertEqual(entry.primaryPath, imagePath)
        XCTAssertEqual(GaryxToolCallPresentation.imageRefs(from: [entry]).map(\.path), [imagePath])
        XCTAssertEqual(
            GaryxToolCallPresentation.detail(for: entry).sections.map(\.content),
            [.codeCard(prompt), .imagePreview(imagePath)]
        )
        XCTAssertFalse(entry.resultText?.contains("iVBORw0KGgo") == true)
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

    func testMissingToolProjectionRendersProviderNeutralFallbackFromCapturedFrame() throws {
        let entries = try mappedToolEntries(fromFrame: #"""
        {
          "type": "thread_render_frame",
          "thread_id": "thread::missing-tool-projection",
          "events": [
            {
              "type": "committed_message",
              "thread_id": "thread::missing-tool-projection",
              "seq": 2,
              "message": {
                "index": 1,
                "role": "tool_use",
                "tool_name": "Bash",
                "tool_use_id": "call-unprojected",
                "timestamp": "2026-07-14T04:00:00Z",
                "content": {
                  "tool": "Bash",
                  "input": {
                    "command": "cat private.txt",
                    "path": "/Users/test/private.png"
                  }
                },
                "metadata": {
                  "source": "claude_sdk",
                  "parent_tool_use_id": "call-parent"
                }
              }
            },
            {
              "type": "committed_message",
              "thread_id": "thread::missing-tool-projection",
              "seq": 3,
              "message": {
                "index": 2,
                "role": "tool_result",
                "tool_name": "Bash",
                "tool_use_id": "call-unprojected",
                "is_error": true,
                "content": { "result": "private output" },
                "metadata": { "source": "claude_sdk" }
              }
            }
          ],
          "render_state": {
            "based_on_seq": 3,
            "rows": [
              {
                "kind": "user_turn",
                "id": "turn:missing-tool-projection",
                "user": null,
                "activity": [
                  {
                    "kind": "step",
                    "id": "step:missing-tool-projection",
                    "steps": [
                      {
                        "kind": "tool_group",
                        "id": "group:missing-tool-projection",
                        "status": "completed",
                        "entries": [
                          {
                            "id": "entry:missing-tool-projection",
                            "tool_use_id": "call-unprojected",
                            "status": "failed",
                            "tool_use": { "id": "seq:2", "seq": 2, "role": "tool_use" },
                            "tool_result": { "id": "seq:3", "seq": 3, "role": "tool_result" }
                          }
                        ]
                      }
                    ],
                    "final_message": null,
                    "running": false
                  }
                ]
              }
            ],
            "tailActivity": "none",
            "activeToolGroupId": null,
            "progress_locus": "none",
            "filtered_placeholders": []
          }
        }
        """#)

        let entry = try XCTUnwrap(entries.only)
        XCTAssertEqual(entry.toolUseId, "call-unprojected")
        XCTAssertNil(entry.parentToolUseId)
        XCTAssertEqual(entry.toolName, "tool")
        XCTAssertEqual(entry.title, "Tool")
        XCTAssertNil(entry.inputText)
        XCTAssertNil(entry.resultText)
        XCTAssertNil(entry.summaryText)
        XCTAssertEqual(entry.inputLabel, "Call")
        XCTAssertEqual(entry.resultLabel, "Result")
        XCTAssertEqual(entry.status, .failed)
        XCTAssertTrue(entry.isError)
        XCTAssertNil(entry.timestamp)
        XCTAssertNil(entry.primaryPathBadge)
        XCTAssertNil(entry.primaryPath)
        XCTAssertNil(entry.fieldProjection)
        XCTAssertTrue(GaryxToolCallPresentation.imageRefs(from: entries).isEmpty)
        let listRow = try XCTUnwrap(GaryxToolCallPresentation.listRows(from: entries).only)
        XCTAssertEqual(listRow.verb, "Tool")
        XCTAssertNil(listRow.detail)
        XCTAssertTrue(GaryxToolCallPresentation.detail(for: entry).sections.isEmpty)
    }

    func testProjectedToolEntryDoesNotMixUnselectedRawPayloadFields() throws {
        let entries = try mappedToolEntries(fromFrame: #"""
        {
          "type": "thread_render_frame",
          "thread_id": "thread::selector-only",
          "events": [
            {
              "type": "committed_message",
              "thread_id": "thread::selector-only",
              "seq": 2,
              "message": {
                "index": 1,
                "role": "tool_use",
                "tool_name": "Bash",
                "tool_use_id": "call-selector-only",
                "timestamp": "2026-07-14T04:00:00Z",
                "content": {
                  "tool": "Bash",
                  "selected": "server-selected call",
                  "input": {
                    "command": "raw command",
                    "path": "/Users/test/raw-private.png"
                  },
                  "is_error": true
                },
                "metadata": {
                  "source": "claude_sdk",
                  "parent_tool_use_id": "call-parent"
                }
              }
            },
            {
              "type": "committed_message",
              "thread_id": "thread::selector-only",
              "seq": 3,
              "message": {
                "index": 2,
                "role": "tool_result",
                "tool_name": "Bash",
                "tool_use_id": "call-selector-only",
                "content": {
                  "result": "raw private result",
                  "is_error": true
                },
                "metadata": { "source": "claude_sdk" }
              }
            }
          ],
          "render_state": {
            "based_on_seq": 3,
            "rows": [
              {
                "kind": "user_turn",
                "id": "turn:selector-only",
                "user": null,
                "activity": [
                  {
                    "kind": "step",
                    "id": "step:selector-only",
                    "steps": [
                      {
                        "kind": "tool_group",
                        "id": "group:selector-only",
                        "status": "completed",
                        "entries": [
                          {
                            "id": "entry:selector-only",
                            "tool_use_id": "call-selector-only",
                            "status": "completed",
                            "tool_use": { "id": "seq:2", "seq": 2, "role": "tool_use" },
                            "tool_result": { "id": "seq:3", "seq": 3, "role": "tool_result" },
                            "projection": {
                              "kind": "generic",
                              "visibility": "normal",
                              "call": {
                                "root": "content",
                                "path": ["selected"],
                                "format": "text",
                                "label": "call"
                              },
                              "status": "completed"
                            }
                          }
                        ]
                      }
                    ],
                    "final_message": null,
                    "running": false
                  }
                ]
              }
            ],
            "tailActivity": "none",
            "activeToolGroupId": null,
            "progress_locus": "none",
            "filtered_placeholders": []
          }
        }
        """#)

        let entry = try XCTUnwrap(entries.only)
        XCTAssertEqual(entry.toolUseId, "call-selector-only")
        XCTAssertNil(entry.parentToolUseId)
        XCTAssertEqual(entry.toolName, "tool")
        XCTAssertEqual(entry.title, "Tool")
        XCTAssertEqual(entry.inputText, "server-selected call")
        XCTAssertEqual(entry.summaryText, "server-selected call")
        XCTAssertNil(entry.resultText)
        XCTAssertFalse(entry.isError)
        XCTAssertNil(entry.timestamp)
        XCTAssertNil(entry.primaryPathBadge)
        XCTAssertNil(entry.primaryPath)
        XCTAssertNotNil(entry.fieldProjection)
        XCTAssertTrue(GaryxToolCallPresentation.imageRefs(from: entries).isEmpty)
    }

    func testCapturedToolFrameResolvesCommandPathAndImageSelectors() throws {
        let entries = try mappedToolEntries(fromFrame: #"""
        {
          "type": "thread_render_frame",
          "thread_id": "thread::tool-selector-matrix",
          "events": [
            {
              "type": "committed_message",
              "thread_id": "thread::tool-selector-matrix",
              "seq": 2,
              "message": {
                "index": 1,
                "role": "tool_use",
                "tool_name": "Read",
                "tool_use_id": "call-read",
                "content": {
                  "tool": "Read",
                  "input": {
                    "file_path": "/Users/test/repo/README.md",
                    "ignored": "raw call noise"
                  }
                },
                "metadata": { "source": "claude_sdk" }
              }
            },
            {
              "type": "committed_message",
              "thread_id": "thread::tool-selector-matrix",
              "seq": 3,
              "message": {
                "index": 2,
                "role": "tool_result",
                "tool_name": "Read",
                "tool_use_id": "call-read",
                "content": {
                  "result": "captured read output",
                  "text": "captured read output",
                  "ignored": "raw result noise"
                },
                "metadata": { "source": "claude_sdk" }
              }
            },
            {
              "type": "committed_message",
              "thread_id": "thread::tool-selector-matrix",
              "seq": 4,
              "message": {
                "index": 3,
                "role": "tool_use",
                "tool_name": "commandExecution",
                "tool_use_id": "call-command",
                "content": {
                  "type": "commandExecution",
                  "command": "/bin/zsh -lc \"git status --short\"",
                  "cwd": "/Users/test/repo",
                  "id": "exec-test"
                },
                "metadata": {
                  "source": "codex_app_server",
                  "item_type": "commandExecution"
                }
              }
            },
            {
              "type": "committed_message",
              "thread_id": "thread::tool-selector-matrix",
              "seq": 5,
              "message": {
                "index": 4,
                "role": "tool_result",
                "tool_name": "commandExecution",
                "tool_use_id": "call-command",
                "content": {
                  "type": "commandExecution",
                  "aggregatedOutput": "fatal: test failure\n",
                  "cwd": "/Users/test/repo",
                  "id": "exec-test",
                  "status": "failed",
                  "exitCode": 17,
                  "durationMs": 12
                },
                "metadata": {
                  "source": "codex_app_server",
                  "item_type": "commandExecution"
                }
              }
            },
            {
              "type": "committed_message",
              "thread_id": "thread::tool-selector-matrix",
              "seq": 6,
              "message": {
                "index": 5,
                "role": "tool_use",
                "tool_name": "imageView",
                "tool_use_id": "call-image",
                "content": {
                  "id": "exec-image",
                  "path": "/tmp/screens/thread-runtime-expanded.png",
                  "type": "ImageView"
                },
                "metadata": { "source": "codex_app_server" }
              }
            },
            {
              "type": "committed_message",
              "thread_id": "thread::tool-selector-matrix",
              "seq": 7,
              "message": {
                "index": 6,
                "role": "tool_result",
                "tool_name": "imageView",
                "tool_use_id": "call-image",
                "content": {
                  "id": "exec-image",
                  "path": "/tmp/screens/thread-runtime-expanded.png",
                  "type": "ImageView"
                },
                "metadata": { "source": "codex_app_server" }
              }
            },
            {
              "type": "committed_message",
              "thread_id": "thread::tool-selector-matrix",
              "seq": 8,
              "message": {
                "index": 7,
                "role": "tool_use",
                "tool_name": "commandExecution",
                "tool_use_id": "call-no-result-selector",
                "content": {
                  "type": "commandExecution",
                  "command": "true"
                },
                "metadata": { "source": "codex_app_server" }
              }
            },
            {
              "type": "committed_message",
              "thread_id": "thread::tool-selector-matrix",
              "seq": 9,
              "message": {
                "index": 8,
                "role": "tool_result",
                "tool_name": "commandExecution",
                "tool_use_id": "call-no-result-selector",
                "content": {
                  "type": "commandExecution",
                  "aggregatedOutput": null,
                  "cwd": "/Users/test/repo",
                  "id": "exec-private",
                  "status": "completed",
                  "exitCode": 0
                },
                "metadata": { "source": "codex_app_server" }
              }
            }
          ],
          "render_state": {
            "based_on_seq": 9,
            "rows": [
              {
                "kind": "user_turn",
                "id": "turn:tool-selector-matrix",
                "user": null,
                "activity": [
                  {
                    "kind": "step",
                    "id": "step:tool-selector-matrix",
                    "steps": [
                      {
                        "kind": "tool_group",
                        "id": "group:tool-selector-matrix",
                        "status": "completed",
                        "entries": [
                          {
                            "id": "entry:read",
                            "tool_use_id": "call-read",
                            "status": "completed",
                            "tool_use": { "id": "seq:2", "seq": 2, "role": "tool_use" },
                            "tool_result": { "id": "seq:3", "seq": 3, "role": "tool_result" },
                            "projection": {
                              "tool_name": "Read",
                              "kind": "file_read",
                              "visibility": "normal",
                              "call": {
                                "root": "content",
                                "path": ["input", "file_path"],
                                "format": "path",
                                "label": "file"
                              },
                              "result": {
                                "root": "content",
                                "path": ["result"],
                                "format": "text",
                                "label": "result"
                              }
                            }
                          },
                          {
                            "id": "entry:command",
                            "tool_use_id": "call-command",
                            "status": "completed",
                            "tool_use": { "id": "seq:4", "seq": 4, "role": "tool_use" },
                            "tool_result": { "id": "seq:5", "seq": 5, "role": "tool_result" },
                            "projection": {
                              "tool_name": "commandExecution",
                              "kind": "command",
                              "visibility": "normal",
                              "call": {
                                "root": "content",
                                "path": ["command"],
                                "format": "code",
                                "label": "command"
                              },
                              "result": {
                                "root": "content",
                                "path": ["aggregatedOutput"],
                                "format": "code",
                                "label": "output"
                              },
                              "status": "failed",
                              "exit_code": 17,
                              "duration_ms": 12
                            }
                          },
                          {
                            "id": "entry:image",
                            "tool_use_id": "call-image",
                            "status": "completed",
                            "tool_use": { "id": "seq:6", "seq": 6, "role": "tool_use" },
                            "tool_result": { "id": "seq:7", "seq": 7, "role": "tool_result" },
                            "projection": {
                              "tool_name": "imageView",
                              "kind": "image",
                              "visibility": "normal",
                              "call": {
                                "root": "content",
                                "path": ["path"],
                                "format": "image",
                                "label": "image"
                              }
                            }
                          },
                          {
                            "id": "entry:no-result-selector",
                            "tool_use_id": "call-no-result-selector",
                            "status": "completed",
                            "tool_use": { "id": "seq:8", "seq": 8, "role": "tool_use" },
                            "tool_result": { "id": "seq:9", "seq": 9, "role": "tool_result" },
                            "projection": {
                              "tool_name": "commandExecution",
                              "kind": "command",
                              "visibility": "normal",
                              "call": {
                                "root": "content",
                                "path": ["command"],
                                "format": "code",
                                "label": "command"
                              },
                              "status": "completed",
                              "exit_code": 0
                            }
                          }
                        ]
                      }
                    ],
                    "final_message": null,
                    "running": false
                  }
                ]
              }
            ],
            "tailActivity": "none",
            "activeToolGroupId": null,
            "progress_locus": "none",
            "filtered_placeholders": []
          }
        }
        """#)

        XCTAssertEqual(entries.count, 4)
        let entriesById = Dictionary(uniqueKeysWithValues: entries.map { ($0.id, $0) })

        let read = try XCTUnwrap(entriesById["entry:read"])
        XCTAssertEqual(read.title, "Read")
        XCTAssertEqual(read.inputText, "/Users/test/repo/README.md")
        XCTAssertEqual(read.resultText, "captured read output")
        XCTAssertEqual(read.primaryPath, "/Users/test/repo/README.md")
        XCTAssertEqual(read.primaryPathBadge, "repo/README.md")

        let command = try XCTUnwrap(entriesById["entry:command"])
        XCTAssertEqual(command.title, "Command")
        XCTAssertEqual(command.inputText, #"/bin/zsh -lc "git status --short""#)
        XCTAssertEqual(command.resultText, "fatal: test failure\n")
        XCTAssertEqual(command.fieldProjection?.metadataText, "exit 17 · 12 ms")
        XCTAssertTrue(command.isError)
        XCTAssertFalse(command.resultText?.contains("/Users/test/repo") == true)
        XCTAssertFalse(command.resultText?.contains("exec-test") == true)

        let image = try XCTUnwrap(entriesById["entry:image"])
        let imagePath = "/tmp/screens/thread-runtime-expanded.png"
        XCTAssertEqual(image.title, "Image")
        XCTAssertEqual(image.inputText, imagePath)
        XCTAssertNil(image.resultText)
        XCTAssertEqual(image.primaryPath, imagePath)
        XCTAssertEqual(GaryxToolCallPresentation.imageRefs(from: [image]).map(\.path), [imagePath])
        XCTAssertEqual(
            GaryxToolCallPresentation.detail(for: image).sections.map(\.content),
            [.imagePreview(imagePath)]
        )

        let noResult = try XCTUnwrap(entriesById["entry:no-result-selector"])
        XCTAssertEqual(noResult.inputText, "true")
        XCTAssertNil(noResult.resultText)
        XCTAssertNil(noResult.fieldProjection?.result)
        XCTAssertEqual(GaryxToolCallPresentation.detail(for: noResult).sections.map(\.label), ["Command"])
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
        // Live committed users keep the same origin identity as the local
        // optimistic row and the server render ref.
        XCTAssertEqual(committed.map(\.id), ["origin:\(origin)"])
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

    func testFileChangeBodySharedFixtureMapsWriteAndEditDiffsEndToEnd() throws {
        let snapshot = try JSONDecoder().decode(
            GaryxRenderSnapshot.self,
            from: Data(contentsOf: fixtureURL(
                directory: "render-layer",
                name: "file-change-body-render-state.json"
            ))
        )
        let transcriptMessages = try fixtureTranscriptMessages(
            directory: "render-layer",
            name: "file-change-body-transcript.jsonl"
        )
        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: GaryxMobileTranscriptMapper.mobileMessages(from: transcriptMessages),
            transcriptMessages: transcriptMessages
        )
        let toolMessage = try XCTUnwrap(toolGroupMessages(in: rows).only)
        let entries = try XCTUnwrap(toolMessage.toolTraceGroup).entries
        XCTAssertEqual(entries.map(\.title), ["Write", "Edit"])
        XCTAssertEqual(entries.map(\.primaryPath), [
            "/Users/test/repo/Sample.txt",
            "/Users/test/repo/Sample.txt",
        ])
        XCTAssertEqual(entries.map(\.primaryPathBadge), ["repo/Sample.txt", "repo/Sample.txt"])
        XCTAssertEqual(toolMessage.toolTraceGroup?.summary, "Edited 1 file")

        let writeDiff = try XCTUnwrap(entries[0].fieldProjection).diff
        XCTAssertEqual(writeDiff.map(\.kind), [.added, .added, .added])
        XCTAssertEqual(writeDiff.map(\.text), ["alpha", "beta", ""])
        let editDiff = try XCTUnwrap(entries[1].fieldProjection).diff
        XCTAssertEqual(editDiff.map(\.kind), [
            .removed, .removed, .removed,
            .added, .added, .added,
        ])
        XCTAssertEqual(editDiff.map(\.text), [
            "alpha", "beta", "",
            "alpha", "gamma", "",
        ])
        XCTAssertEqual(
            entries.map { GaryxToolCallPresentation.detail(for: $0).sections.map(\.label) },
            [["File", "Diff", "Result"], ["File", "Diff", "Result"]]
        )
    }

    func testPathSummaryDrivesPrimaryPathFileCountingAndWrittenImageRefs() throws {
        let transcriptMessages = [
            GaryxTranscriptMessage(
                index: 0,
                role: .toolUse,
                content: json(#"{"tool":"Write","input":{"file_path":"/Users/test/repo/generated.png","content":"pixels"}}"#)
            ),
            GaryxTranscriptMessage(
                index: 1,
                role: .toolUse,
                content: json(#"{"tool":"Edit","input":{"file_path":"/Users/test/repo/Other.swift","old_string":"a","new_string":"b"}}"#)
            ),
        ]
        let pathSelector: (String) -> GaryxRenderToolFieldSelector = { toolField in
            GaryxRenderToolFieldSelector(
                root: .content,
                path: ["input", toolField],
                format: .path,
                label: .file
            )
        }
        let valueSelector: (String) -> GaryxRenderToolValueSelector = { toolField in
            GaryxRenderToolValueSelector(root: .content, path: ["input", toolField])
        }
        let snapshot = GaryxRenderSnapshot(
            basedOnSeq: 2,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "turn:path-summary",
                    user: nil,
                    activity: [
                        .step(GaryxRenderStepRow(
                            id: "step:path-summary",
                            steps: [
                                .toolGroup(GaryxRenderToolGroup(
                                    id: "group:path-summary",
                                    status: .completed,
                                    entries: [
                                        GaryxRenderToolEntry(
                                            id: "entry:write-image",
                                            status: .completed,
                                            toolUse: ref(seq: 1, role: "tool_use"),
                                            projection: GaryxRenderToolFieldProjection(
                                                toolName: "Write",
                                                kind: .fileWrite,
                                                summary: pathSelector("file_path"),
                                                diff: GaryxRenderToolDiffRecipe(
                                                    source: .toolUse,
                                                    segments: [.pair(old: nil, new: valueSelector("content"))]
                                                )
                                            )
                                        ),
                                        GaryxRenderToolEntry(
                                            id: "entry:edit-source",
                                            status: .completed,
                                            toolUse: ref(seq: 2, role: "tool_use"),
                                            projection: GaryxRenderToolFieldProjection(
                                                toolName: "Edit",
                                                kind: .fileEdit,
                                                summary: pathSelector("file_path"),
                                                diff: GaryxRenderToolDiffRecipe(
                                                    source: .toolUse,
                                                    segments: [
                                                        .pair(
                                                            old: valueSelector("old_string"),
                                                            new: valueSelector("new_string")
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

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: [],
            transcriptMessages: transcriptMessages
        )
        let toolMessage = try XCTUnwrap(toolGroupMessages(in: rows).only)
        let entries = try XCTUnwrap(toolMessage.toolTraceGroup).entries
        XCTAssertEqual(entries.map(\.primaryPath), [
            "/Users/test/repo/generated.png",
            "/Users/test/repo/Other.swift",
        ])
        XCTAssertEqual(entries.map(\.primaryPathBadge), ["repo/generated.png", "repo/Other.swift"])
        XCTAssertEqual(toolMessage.toolTraceGroup?.summary, "Edited 2 files")
        XCTAssertEqual(
            GaryxToolCallPresentation.imageRefs(from: entries).map(\.path),
            ["/Users/test/repo/generated.png"]
        )
    }

    func testFrozenPreChangeDecoderIgnoresNewDiffAndPinsStaleMobilePresentation() throws {
        let rawState = try JSONSerialization.jsonObject(
            with: Data(contentsOf: fixtureURL(
                directory: "render-layer",
                name: "file-change-body-render-state.json"
            ))
        ) as? [String: Any]
        let rows = try XCTUnwrap(rawState?["rows"] as? [[String: Any]])
        let activity = try XCTUnwrap(rows.only?["activity"] as? [[String: Any]])
        let steps = try XCTUnwrap(activity.only?["steps"] as? [[String: Any]])
        let entries = try XCTUnwrap(steps.only?["entries"] as? [[String: Any]])
        let frozenEntries = try entries.map { entry in
            try JSONDecoder().decode(
                FrozenPreChangeToolEntry.self,
                from: JSONSerialization.data(withJSONObject: entry)
            )
        }
        XCTAssertEqual(frozenEntries.map(\.projection.kind), [.fileWrite, .fileEdit])
        XCTAssertEqual(frozenEntries.map(\.projection.summary?.format), [.path, .path])
        XCTAssertTrue(frozenEntries.allSatisfy { $0.projection.call == nil })
        XCTAssertEqual(frozenEntries.map(\.projection.result?.label), [.result, .result])

        let fileChange = try JSONDecoder().decode(
            FrozenPreChangeToolEntry.self,
            from: Data(#"""
            {
              "id": "entry:file-change",
              "status": "completed",
              "projection": {
                "tool_name": "fileChange",
                "kind": "file_edit",
                "visibility": "normal",
                "diff": {
                  "source": "tool_use",
                  "segments": [
                    { "unified": { "text": { "root": "content", "path": ["changes", "0", "diff"] } } }
                  ]
                }
              }
            }
            """#.utf8)
        )
        XCTAssertEqual(fileChange.projection.kind, .fileEdit)
        XCTAssertNil(fileChange.projection.summary)
        XCTAssertNil(fileChange.projection.call)
        XCTAssertNil(fileChange.projection.result)
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

    private func mappedToolEntries(fromFrame raw: String) throws -> [GaryxMobileToolTraceEntry] {
        let frame = try JSONDecoder().decode(GaryxThreadRenderFrame.self, from: Data(raw.utf8))
        let transcriptMessages = frame.events.compactMap(\.message)
        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: frame.renderState,
            messages: GaryxMobileTranscriptMapper.mobileMessages(from: transcriptMessages),
            transcriptMessages: transcriptMessages
        )
        var entries: [GaryxMobileToolTraceEntry] = []
        for row in rows {
            for activity in row.activityRows {
                guard case let .turn(turn) = activity else { continue }
                for step in turn.steps {
                    guard case let .toolGroup(message) = step,
                          let group = message.toolTraceGroup else { continue }
                    entries.append(contentsOf: group.entries)
                }
            }
        }
        return entries
    }

    private func fixtureURL(directory: String, name: String) -> URL {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 {
            url.deleteLastPathComponent()
        }
        return url
            .appendingPathComponent("test-fixtures")
            .appendingPathComponent(directory)
            .appendingPathComponent(name)
    }

    private func fixtureTranscriptMessages(
        directory: String,
        name: String
    ) throws -> [GaryxTranscriptMessage] {
        try String(contentsOf: fixtureURL(directory: directory, name: name))
            .split(whereSeparator: \.isNewline)
            .map(String.init)
            .filter { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            .map { line in
                let record = try XCTUnwrap(
                    JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any]
                )
                let rawMessage = try XCTUnwrap(record["message"] as? [String: Any])
                var message = try JSONDecoder().decode(
                    GaryxTranscriptMessage.self,
                    from: JSONSerialization.data(withJSONObject: rawMessage)
                )
                if let seq = record["seq"] as? Int {
                    message.index = seq - 1
                    message.id = "history:\(seq - 1)"
                }
                return message
            }
    }

    private func toolGroupMessages(in rows: [GaryxMobileTurnRow]) -> [GaryxMobileMessage] {
        rows.flatMap(\.activityRows).flatMap { activity -> [GaryxMobileMessage] in
            guard case .turn(let turn) = activity else { return [] }
            return turn.steps.compactMap { step in
                guard case .toolGroup(let message) = step else { return nil }
                return message
            }
        }
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

private struct FrozenPreChangeToolEntry: Decodable {
    let id: String
    let status: String
    let projection: FrozenPreChangeProjection
}

/// Exact presentation/array strictness of the app immediately before the
/// structured-presentation cutover. This is validation of the accepted rollout
/// failure shape, not a compatibility decoder used by the product.
private struct FrozenStringPresentationFrame: Decodable {
    let renderState: FrozenStringPresentationSnapshot?
    let renderDelta: FrozenStringPresentationDeltaPayload?

    enum CodingKeys: String, CodingKey {
        case renderState = "render_state"
        case renderDelta = "render_delta"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        renderState = try container.decodeIfPresent(FrozenStringPresentationSnapshot.self, forKey: .renderState)
        do {
            renderDelta = try container.decodeIfPresent(
                FrozenStringPresentationDelta.self,
                forKey: .renderDelta
            ).map(FrozenStringPresentationDeltaPayload.delta)
        } catch is DecodingError {
            renderDelta = .malformed
        }
    }

    var deltaResolution: FrozenStringPresentationDeltaResolution {
        guard let renderDelta else { return .ignored }
        guard case .delta = renderDelta else { return .gapReplay }
        return .applied
    }
}

private struct FrozenStringPresentationSnapshot: Decodable {
    let rows: [FrozenStringPresentationRow]

    enum CodingKeys: String, CodingKey {
        case rows
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        rows = try container.decode(
            FrozenStringPresentationLossyArray<FrozenStringPresentationRow>.self,
            forKey: .rows
        ).elements
    }
}

private struct FrozenStringPresentationDelta: Decodable {
    let upsertRows: [FrozenStringPresentationRow]

    enum CodingKeys: String, CodingKey {
        case upsertRows = "upsert_rows"
    }
}

private enum FrozenStringPresentationDeltaPayload {
    case delta(FrozenStringPresentationDelta)
    case malformed
}

private enum FrozenStringPresentationDeltaResolution: Equatable {
    case applied
    case gapReplay
    case ignored
}

private struct FrozenStringPresentationRow: Decodable {
    let id: String
    let user: FrozenStringPresentationMessageRef
}

private struct FrozenStringPresentationMessageRef: Decodable {
    let id: String
    let seq: Int
    let role: String
    let presentation: String?
}

private struct FrozenStringPresentationLossyArray<Element: Decodable>: Decodable {
    let elements: [Element]

    init(from decoder: Decoder) throws {
        var container = try decoder.unkeyedContainer()
        var elements: [Element] = []
        while !container.isAtEnd {
            if let element = try? container.decode(Element.self) {
                elements.append(element)
            } else {
                _ = try? container.decode(FrozenStringPresentationDiscard.self)
            }
        }
        self.elements = elements
    }
}

private struct FrozenStringPresentationDiscard: Decodable {}

private struct FrozenPreChangeProjection: Decodable {
    enum Kind: String, Decodable {
        case fileWrite = "file_write"
        case fileEdit = "file_edit"
    }

    let toolName: String?
    let kind: Kind
    let summary: FrozenPreChangeSelector?
    let call: FrozenPreChangeSelector?
    let result: FrozenPreChangeSelector?

    enum CodingKeys: String, CodingKey {
        case toolName = "tool_name"
        case kind
        case summary
        case call
        case result
    }
}

private struct FrozenPreChangeSelector: Decodable {
    enum Format: String, Decodable {
        case text
        case code
        case path
        case json
        case diff
        case image
    }

    enum Label: String, Decodable {
        case call
        case file
        case result
    }

    let root: String
    let path: [String]?
    let format: Format
    let label: Label
}

private extension Array {
    var only: Element? {
        count == 1 ? self[0] : nil
    }
}
