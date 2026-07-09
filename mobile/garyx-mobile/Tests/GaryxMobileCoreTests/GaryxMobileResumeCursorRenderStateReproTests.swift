import XCTest
@testable import GaryxMobileCore

/// RED reproduction for the upper half of the latest-user skeleton bug.
///
/// Shape:
/// - the client sees a render_state row for seq 95 without receiving a committed
///   body event for seq 95;
/// - the stream processor nevertheless treats `based_on_seq == 95` as the
///   committed frontier;
/// - a later gap reconnect resumes after 95;
/// - server replay is strictly `seq > after_seq`, so the committed user body at
///   seq 95 is skipped and the render_state ref stays body-less.
final class GaryxMobileResumeCursorRenderStateReproTests: XCTestCase {
    func testRenderStateOnlySeqDoesNotBecomeCommittedResumeCursor() throws {
        let threadId = "thread::resume-cursor-repro"
        let missingUserSeq = 95
        let laterAssistantSeq = 113
        var processor = GatewayStreamFrameProcessor()

        let renderOnly = processor.processPayload(
            framePayload(
                threadId: threadId,
                basedOnSeq: missingUserSeq,
                events: [],
                rows: [userTurnRow(userSeq: missingUserSeq, assistantSeq: nil)]
            ),
            threadId: threadId
        )

        XCTAssertTrue(committedMessages(in: renderOnly.actions).isEmpty)
        XCTAssertEqual(renderSnapshot(in: renderOnly.actions)?.basedOnSeq, missingUserSeq)
        XCTAssertEqual(
            processor.connectionLastSeq,
            0,
            "A render_state ref without a committed event must not advance the committed stream frontier."
        )

        let gapAfterRenderOnly = processor.processPayload(
            framePayload(
                threadId: threadId,
                basedOnSeq: laterAssistantSeq,
                events: [
                    committedEvent(seq: laterAssistantSeq, role: "assistant", text: "Test assistant response"),
                ],
                rows: [userTurnRow(userSeq: missingUserSeq, assistantSeq: laterAssistantSeq)]
            ),
            threadId: threadId
        )

        let reconnectAfterSeq = try XCTUnwrap(gapAfterRenderOnly.reconnect?.resumeAfterSeq)
        XCTAssertLessThan(
            reconnectAfterSeq,
            missingUserSeq,
            "Gap reconnect must resume before the unresolved user seq, not after the render_state-only seq."
        )

        let replayedSeqs = serverReplaySeqs(
            committedSeqs: [missingUserSeq, laterAssistantSeq],
            afterSeq: reconnectAfterSeq
        )
        XCTAssertTrue(
            replayedSeqs.contains(missingUserSeq),
            "Because server replay is seq > after_seq, after_seq \(reconnectAfterSeq) permanently skips user seq \(missingUserSeq)."
        )

        let replayedMessages = replayedSeqs.map { seq in
            GaryxTranscriptMessage(
                index: seq - 1,
                role: seq == missingUserSeq ? .user : .assistant,
                text: seq == missingUserSeq ? "Test user prompt" : "Test assistant response"
            )
        }
        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: latestTurnSnapshot(userSeq: missingUserSeq, assistantSeq: laterAssistantSeq),
            messages: GaryxMobileTranscriptMapper.mobileMessages(from: replayedMessages),
            transcriptMessages: replayedMessages
        )
        let user = try XCTUnwrap(rows.only?.userBlock?.message)
        XCTAssertNotEqual(
            GaryxMobileMessagePresentation.make(for: user),
            .historySkeleton,
            "Once replay is correctly anchored before seq \(missingUserSeq), the user body is available and the row cannot remain a loading skeleton."
        )
    }

    private func committedMessages(in actions: [GatewayStreamAction]) -> [GaryxTranscriptMessage] {
        actions.flatMap { action -> [GaryxTranscriptMessage] in
            guard case let .applyCommittedMessages(messages) = action else { return [] }
            return messages
        }
    }

    private func renderSnapshot(in actions: [GatewayStreamAction]) -> GaryxRenderSnapshot? {
        actions.compactMap { action -> GaryxRenderSnapshot? in
            guard case let .applyRenderSnapshot(snapshot) = action else { return nil }
            return snapshot
        }.last
    }

    private func serverReplaySeqs(committedSeqs: [Int], afterSeq: Int) -> [Int] {
        committedSeqs.filter { $0 > afterSeq }
    }

    private func latestTurnSnapshot(userSeq: Int, assistantSeq: Int) -> GaryxRenderSnapshot {
        GaryxRenderSnapshot(
            basedOnSeq: assistantSeq,
            rows: [
                .userTurn(GaryxRenderUserTurnRow(
                    id: "turn:user:\(userSeq)",
                    user: GaryxRenderMessageRef(id: "seq:\(userSeq)", seq: userSeq, role: "user"),
                    activity: [
                        .step(GaryxRenderStepRow(
                            id: "step:assistant:\(assistantSeq)",
                            steps: [],
                            finalMessage: GaryxRenderMessageRef(
                                id: "seq:\(assistantSeq)",
                                seq: assistantSeq,
                                role: "assistant"
                            ),
                            running: false
                        )),
                    ]
                )),
            ]
        )
    }

    private func framePayload(
        threadId: String,
        basedOnSeq: Int,
        events: [[String: Any]],
        rows: [[String: Any]]
    ) -> String {
        jsonString([
            "type": "thread_render_frame",
            "thread_id": threadId,
            "events": events,
            "render_state": [
                "based_on_seq": basedOnSeq,
                "rows": rows,
                "tailActivity": "none",
                "progress_locus": "none",
                "filtered_placeholders": [],
            ],
        ])
    }

    private func committedEvent(seq: Int, role: String, text: String) -> [String: Any] {
        [
            "type": "committed_message",
            "seq": seq,
            "message": [
                "role": role,
                "text": text,
            ],
        ]
    }

    private func userTurnRow(userSeq: Int, assistantSeq: Int?) -> [String: Any] {
        var activity: [[String: Any]] = []
        if let assistantSeq {
            activity = [
                [
                    "kind": "step",
                    "id": "step:assistant:\(assistantSeq)",
                    "steps": [],
                    "final_message": [
                        "id": "seq:\(assistantSeq)",
                        "seq": assistantSeq,
                        "role": "assistant",
                    ],
                    "running": false,
                ],
            ]
        }
        return [
            "kind": "user_turn",
            "id": "turn:user:\(userSeq)",
            "user": [
                "id": "seq:\(userSeq)",
                "seq": userSeq,
                "role": "user",
            ],
            "activity": activity,
            "capsule_cards": [],
        ]
    }

    private func jsonString(_ object: [String: Any]) -> String {
        let data = try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return String(data: data, encoding: .utf8)!
    }
}

private extension GatewayStreamReconnect {
    var resumeAfterSeq: Int? {
        guard case let .gap(resumeAfterSeq) = self else { return nil }
        return resumeAfterSeq
    }
}

private extension Array {
    var only: Element? {
        count == 1 ? self[0] : nil
    }
}
