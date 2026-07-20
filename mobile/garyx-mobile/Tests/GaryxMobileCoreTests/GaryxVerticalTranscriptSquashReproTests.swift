@testable import GaryxMobileCore
import XCTest

final class GaryxVerticalTranscriptSquashReproTests: XCTestCase {
    /// Scrubbed from the affected thread's production `thread_render_frame`.
    /// The live frame and its settled successor differ in run presentation,
    /// while both reference the same committed assistant body. This pins the
    /// Core boundary before exercising the UIKit snapshot geometry in the app
    /// test target.
    func testCapturedActiveAndSettledFramesKeepAssistantLayoutInputIdentical() throws {
        let active = try mappedAssistant(from: frame(running: true))
        let settled = try mappedAssistant(from: frame(running: false))

        XCTAssertTrue(active.turnIsRunning)
        XCTAssertFalse(settled.turnIsRunning)
        XCTAssertEqual(active.message.id, settled.message.id)
        XCTAssertEqual(active.message.role, .assistant)
        XCTAssertEqual(active.message.text, settled.message.text)
        XCTAssertEqual(
            GaryxMobileMessagePresentation.make(for: active.message),
            GaryxMobileMessagePresentation.make(for: settled.message)
        )
        XCTAssertEqual(
            GaryxMobileMessagePresentation.make(for: active.message),
            .text("会话恢复。逐面提取瞬态界面。\n\n五个页面已经全部提取完成。")
        )
    }

    private func mappedAssistant(
        from raw: String
    ) throws -> (message: GaryxMobileMessage, turnIsRunning: Bool) {
        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 0, replayScope: .resume)
        let result = processor.processPayload(raw, threadId: "thread::layout-input-repro")

        var committed: [GaryxTranscriptMessage] = []
        var snapshot: GaryxRenderSnapshot?
        for action in result.actions {
            if case let .applyCommittedMessages(messages) = action {
                committed = messages
            }
            if case let .applyRenderSnapshot(renderSnapshot) = action {
                snapshot = renderSnapshot
            }
        }

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: try XCTUnwrap(snapshot),
            messages: GaryxMobileTranscriptMapper.mobileMessages(from: committed),
            transcriptMessages: committed
        )
        let row = try XCTUnwrap(rows.first)
        guard case let .turn(turn) = try XCTUnwrap(row.activityRows.first) else {
            throw LayoutInputReproError.expectedAgentTurn
        }
        return (try XCTUnwrap(turn.steps.first?.message), turn.isRunning)
    }

    private func frame(running: Bool) -> String {
        let streaming = running ? "true" : "false"
        let tailActivity = running ? "assistant_streaming" : "none"
        let progressLocus = running ? "tail" : "none"
        return #"""
        {
          "type": "thread_render_frame",
          "thread_id": "thread::layout-input-repro",
          "events": [
            {
              "type": "committed_message",
              "thread_id": "thread::layout-input-repro",
              "seq": 1,
              "message": { "role": "user", "text": "继续处理界面" }
            },
            {
              "type": "committed_message",
              "thread_id": "thread::layout-input-repro",
              "seq": 2,
              "message": {
                "role": "assistant",
                "text": "会话恢复。逐面提取瞬态界面。\n\n五个页面已经全部提取完成。"
              }
            }
          ],
          "render_state": {
            "based_on_seq": 2,
            "rows": [
              {
                "kind": "user_turn",
                "id": "user_turn:seq:1",
                "user": { "id": "seq:1", "seq": 1, "role": "user" },
                "activity": [
                  {
                    "kind": "step",
                    "id": "step:assistant_step:seq:2",
                    "steps": [
                      {
                        "kind": "assistant_message",
                        "id": "assistant_step:seq:2",
                        "message": { "id": "seq:2", "seq": 2, "role": "assistant" },
                        "streaming": \#(streaming)
                      }
                    ],
                    "final_message": null,
                    "running": \#(streaming),
                    "started_at": null,
                    "finished_at": null
                  }
                ],
                "started_at": null,
                "finished_at": null
              }
            ],
            "tailActivity": "\#(tailActivity)",
            "activeToolGroupId": null,
            "progress_locus": "\#(progressLocus)",
            "filtered_placeholders": []
          }
        }
        """#
    }
}

private enum LayoutInputReproError: Error {
    case expectedAgentTurn
}
