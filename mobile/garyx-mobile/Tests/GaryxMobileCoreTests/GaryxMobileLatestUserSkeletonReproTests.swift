import XCTest
@testable import GaryxMobileCore

/// RED reproduction for #TASK-1460.
///
/// Fixture shape was captured from a live affected thread and then sanitized:
/// - render_state.based_on_seq = 116, window.floor_seq = 1
/// - latest user row ref = seq 95 / origin:* id
/// - same-turn assistant final ref = seq 113
/// - replay from after_seq=95 contains assistant bodies after seq 95, but not the
///   user body at seq 95. The authoritative transcript still has that user body
///   at index 94.
final class GaryxMobileLatestUserSkeletonReproTests: XCTestCase {
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
        text: String
    ) -> GaryxTranscriptMessage {
        GaryxTranscriptMessage(index: index, role: role, text: text)
    }
}

private extension Array {
    var only: Element? {
        count == 1 ? self[0] : nil
    }
}
