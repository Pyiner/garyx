import XCTest
@testable import GaryxMobileCore

final class GaryxMobileOldUserTailReproTests: XCTestCase {
    /// Reproduces the captured long-thread failure:
    ///
    /// 1. a local optimistic user row is committed by a live render frame with
    ///    the same `metadata.origin_id`;
    /// 2. the server render window initially represents that row, so the stale
    ///    local copy is hidden;
    /// 3. after the window floor advances past the old turn, the stale local
    ///    copy becomes visible again and is appended after newer server rows.
    ///
    /// The old turn's canonical anchor is seq 11. Once a window starts at seq
    /// 21 it must not reappear at the tail after the seq 21/31 turns.
    func testCommittedOriginRowDoesNotReappearAfterRenderFloorAdvances() throws {
        let oldOrigin = "mobile-00000000-0000-0000-0000-000000000001"
        let firstNewOrigin = "mobile-00000000-0000-0000-0000-000000000002"
        let secondNewOrigin = "mobile-00000000-0000-0000-0000-000000000003"
        let optimisticOldUser = GaryxMobileMessage(
            id: "origin:\(oldOrigin)",
            role: .user,
            text: "Earlier follow-up",
            timestamp: nil,
            isStreaming: false,
            clientIntentId: oldOrigin,
            localState: .optimistic
        )

        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 10, replayScope: .resume)
        let committedFrame = """
        {
          "type": "thread_render_frame",
          "thread_id": "thread::old-user-tail",
          "events": [
            {
              "type": "committed_message",
              "thread_id": "thread::old-user-tail",
              "seq": 11,
              "message": {
                "role": "user",
                "kind": "user_input",
                "text": "Earlier follow-up",
                "metadata": {
                  "origin_id": "\(oldOrigin)",
                  "queued_input_id": "queued_input:synthetic"
                }
              }
            }
          ],
          "render_state": {
            "based_on_seq": 11,
            "rows": [
              {
                "kind": "user_turn",
                "id": "user_turn:origin:\(oldOrigin)",
                "user": { "id": "origin:\(oldOrigin)", "seq": 11, "role": "user" },
                "activity": []
              }
            ]
          }
        }
        """

        let result = processor.processPayload(
            committedFrame,
            threadId: "thread::old-user-tail"
        )
        guard case let .applyCommittedMessages(committedMessages) = try XCTUnwrap(result.actions.first),
              case let .applyRenderSnapshot(initialSnapshot) = try XCTUnwrap(result.actions.last) else {
            return XCTFail("the captured frame must emit its body before its render snapshot")
        }

        let remoteOldUser = GaryxMobileTranscriptMapper.mobileMessages(from: committedMessages)
        let messagesAfterCommit = GaryxTranscriptMerge.mergedMessages(
            remoteOldUser,
            withLocal: [optimisticOldUser]
        )
        XCTAssertEqual(
            GaryxMobileRenderStateMapper.rows(
                snapshot: initialSnapshot,
                messages: messagesAfterCommit,
                transcriptMessages: committedMessages
            ).map(\.id),
            ["user_turn:origin:\(oldOrigin)"],
            "while represented, the committed and optimistic copies are visually deduplicated"
        )

        let firstNewUser = committedUser(origin: firstNewOrigin, seq: 21, text: "Newer question")
        let secondNewUser = committedUser(origin: secondNewOrigin, seq: 31, text: "Newest question")
        let laterSnapshot = GaryxRenderSnapshot(
            basedOnSeq: 31,
            rows: [
                userTurn(origin: firstNewOrigin, seq: 21),
                userTurn(origin: secondNewOrigin, seq: 31),
            ],
            window: GaryxRenderWindow(floorSeq: 21, hasMoreAbove: true)
        )

        let laterRows = GaryxMobileRenderStateMapper.rows(
            snapshot: laterSnapshot,
            messages: messagesAfterCommit + [firstNewUser, secondNewUser],
            transcriptMessages: committedMessages
        )

        XCTAssertEqual(
            laterRows.map(\.id),
            [
                "user_turn:origin:\(firstNewOrigin)",
                "user_turn:origin:\(secondNewOrigin)",
            ],
            "the seq-11 user must not be appended after newer server-owned rows when it falls above the render floor"
        )
    }

    private func committedUser(origin: String, seq: Int, text: String) -> GaryxMobileMessage {
        GaryxMobileMessage(
            id: "origin:\(origin)",
            role: .user,
            text: text,
            timestamp: nil,
            isStreaming: false,
            clientIntentId: origin,
            localState: .remoteFinal,
            historyIndex: seq - 1
        )
    }

    private func userTurn(origin: String, seq: Int) -> GaryxRenderRow {
        .userTurn(GaryxRenderUserTurnRow(
            id: "user_turn:origin:\(origin)",
            user: GaryxRenderMessageRef(id: "origin:\(origin)", seq: seq, role: "user"),
            activity: []
        ))
    }
}
