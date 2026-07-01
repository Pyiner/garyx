import XCTest
@testable import GaryxMobileCore

final class GaryxMobileLatestUserSkeletonReproTests: XCTestCase {
    /// RED reproduction for #TASK-1502.
    ///
    /// Sanitized from the affected scroll-up shape:
    /// - REST history contains an older user row body at index 84.
    /// - That REST projection marks the user row `tool_related=true` with
    ///   `kind=user_input`.
    /// - The server render snapshot later references the same user by seq 85.
    /// - The assistant final body in the same turn resolves normally.
    ///
    /// Current bug: the transcript-to-mobile mapper treats `user_input` as a
    /// tool-use row when `tool_related=true`, drops the user body, and the
    /// render-state mapper falls back to a user loading skeleton.
    func testScrollUpRenderFloorKeepsToolRelatedUserBodiesFromHistory() throws {
        let snapshot = try decodeRenderSnapshot(sanitizedScrollUpRenderState)
        let transcriptMessages = [
            transcriptMessage(
                index: 84,
                role: .user,
                kind: "user_input",
                text: "Synthetic older user prompt",
                toolRelated: true,
                originId: "mobile-TESTDEVICE0001"
            ),
            transcriptMessage(
                index: 89,
                role: .assistant,
                kind: "assistant_reply",
                text: "Synthetic assistant response"
            ),
        ]

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: GaryxMobileTranscriptMapper.mobileMessages(from: transcriptMessages),
            transcriptMessages: transcriptMessages
        )

        let turn = try XCTUnwrap(rows.only)
        let userMessage = try XCTUnwrap(turn.userBlock?.message)
        guard case let .turn(agentTurn) = try XCTUnwrap(turn.activityRows.only) else {
            return XCTFail("same-turn assistant activity should render as an agent turn")
        }
        let finalMessage = try XCTUnwrap(agentTurn.finalBlock?.message)

        XCTAssertEqual(finalMessage.text, "Synthetic assistant response")
        XCTAssertEqual(
            GaryxMobileMessagePresentation.make(for: finalMessage),
            .text("Synthetic assistant response")
        )

        XCTAssertEqual(
            userMessage.text,
            "Synthetic older user prompt",
            "A server-rendered user ref whose committed REST body is present must resolve even when the history projection marks the user row tool_related."
        )
        XCTAssertNotEqual(GaryxMobileMessagePresentation.make(for: userMessage), .historySkeleton)
    }

    /// RED reproduction for #TASK-1460.
    ///
    /// Fixture shape was captured from a live affected thread and then sanitized:
    /// - render_state.based_on_seq = 116, window.floor_seq = 1
    /// - latest user row ref = seq 95 / origin:* id
    /// - same-turn assistant final ref = seq 113
    /// - replay from after_seq=95 contains assistant bodies after seq 95, but not the
    ///   user body at seq 95. The authoritative transcript still has that user body
    ///   at index 94.
    func testLatestUserTurnDoesNotStaySkeletonWhenSameTurnAssistantFinalBodyIsRendered() throws {
        let snapshot = try decodeRenderSnapshot(sanitizedLatestTurnRenderState)
        let messages = [
            // Shape of the mobile message cache after replaying from after_seq=95:
            // assistant final body is present, but user seq 95 / index 94 is not.
            mobileMessage(index: 112, role: .assistant, text: "Test assistant response"),
        ]
        let transcriptMessages = [
            // Authoritative history still has the missing user body at seq 95
            // (history index 94), so this is not a render-floor trim.
            transcriptMessage(index: 94, role: .user, text: "Test user prompt"),
            transcriptMessage(index: 112, role: .assistant, text: "Test assistant response"),
        ]

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: messages,
            transcriptMessages: transcriptMessages
        )

        let latestTurn = try XCTUnwrap(rows.only)
        let userMessage = try XCTUnwrap(latestTurn.userBlock?.message)
        guard case let .turn(agentTurn) = try XCTUnwrap(latestTurn.activityRows.only) else {
            return XCTFail("same-turn assistant activity should render as an agent turn")
        }
        let finalMessage = try XCTUnwrap(agentTurn.finalBlock?.message)

        XCTAssertEqual(finalMessage.text, "Test assistant response")
        XCTAssertEqual(
            GaryxMobileMessagePresentation.make(for: finalMessage),
            .text("Test assistant response")
        )

        XCTAssertEqual(
            userMessage.text,
            "Test user prompt",
            "Latest user body exists in authoritative history at index 94; it must not render as a permanent loading skeleton while the same-turn assistant final body is already present."
        )
        XCTAssertNotEqual(GaryxMobileMessagePresentation.make(for: userMessage), .historySkeleton)
    }

    private var sanitizedScrollUpRenderState: String {
        #"""
        {
          "based_on_seq": 91,
          "rows": [
            {
              "kind": "user_turn",
              "id": "user_turn:origin:mobile-TESTDEVICE0001",
              "user": {
                "id": "origin:mobile-TESTDEVICE0001",
                "role": "user",
                "seq": 85
              },
              "activity": [
                {
                  "kind": "step",
                  "id": "step:assistant_step:seq:90",
                  "steps": [],
                  "final_message": {
                    "id": "seq:90",
                    "role": "assistant",
                    "seq": 90
                  },
                  "running": false,
                  "started_at": "2026-01-01T00:00:00Z",
                  "finished_at": "2026-01-01T00:00:12Z"
                }
              ],
              "capsule_cards": []
            }
          ],
          "window": {
            "floor_seq": 85,
            "has_more_above": true
          }
        }
        """#
    }

    private var sanitizedLatestTurnRenderState: String {
        #"""
        {
          "based_on_seq": 116,
          "rows": [
            {
              "kind": "user_turn",
              "id": "user_turn:origin:mobile-00000000-0000-0000-0000-000000000095",
              "user": {
                "id": "origin:mobile-00000000-0000-0000-0000-000000000095",
                "role": "user",
                "seq": 95
              },
              "activity": [
                {
                  "kind": "step",
                  "id": "step:assistant_step:seq:97",
                  "steps": [],
                  "final_message": {
                    "id": "seq:113",
                    "role": "assistant",
                    "seq": 113
                  },
                  "running": false,
                  "started_at": "2026-01-01T00:00:00Z",
                  "finished_at": "2026-01-01T00:00:12Z"
                }
              ],
              "capsule_cards": []
            }
          ],
          "window": {
            "floor_seq": 1,
            "has_more_above": false
          }
        }
        """#
    }

    private func decodeRenderSnapshot(_ raw: String) throws -> GaryxRenderSnapshot {
        try JSONDecoder().decode(GaryxRenderSnapshot.self, from: Data(raw.utf8))
    }

    private func mobileMessage(
        index: Int,
        role: GaryxMobileMessage.Role,
        text: String
    ) -> GaryxMobileMessage {
        GaryxMobileMessage(
            id: "history:\(index)",
            role: role,
            text: text,
            timestamp: nil,
            isStreaming: false,
            localState: .remoteFinal,
            historyIndex: index
        )
    }

    private func transcriptMessage(
        index: Int,
        role: GaryxTranscriptRole,
        kind: String? = nil,
        text: String,
        toolRelated: Bool = false,
        originId: String? = nil
    ) -> GaryxTranscriptMessage {
        var metadata: [String: GaryxJSONValue] = [:]
        if let originId {
            metadata["origin_id"] = .string(originId)
            metadata["client"] = .string("garyx-mobile")
        }
        var message: [String: GaryxJSONValue] = [
            "role": .string(role.rawValue),
            "content": .string(text),
            "text": .string(text),
        ]
        if !metadata.isEmpty {
            message["metadata"] = .object(metadata)
        }
        return GaryxTranscriptMessage(
            index: index,
            role: role,
            kind: kind,
            text: text,
            content: .string(text),
            message: .object(message),
            toolRelated: toolRelated,
            metadata: metadata.isEmpty ? nil : .object(metadata)
        )
    }
}

private extension Array {
    var only: Element? {
        count == 1 ? self[0] : nil
    }
}
