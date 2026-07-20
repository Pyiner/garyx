import Foundation
import XCTest
@testable import GaryxMobileCore

final class GaryxMessageSendJitterReproTests: XCTestCase {
    /// Sanitized reduction of the live mobile report captured at committed seq
    /// 372. The wire frame had 21 existing turns followed by the just-sent
    /// origin-bearing user turn. This pins both halves of the symptom:
    ///
    /// - optimistic -> committed keeps the exact same outer row and message id;
    /// - the TASK-2523 baseline emitted a second programmatic tail-scroll
    ///   request for that identity-preserving materialization, after the append.
    ///
    /// The last assertion failed on the TASK-2523 baseline and stays as the
    /// regression gate for identity-preserving materialization.
    func testCapturedOriginMaterializationDoesNotScheduleSecondTailReanchor() throws {
        let origin = "mobile-00000000-0000-0000-0000-000000002523"
        let threadScope = "thread:send-jitter-capture"
        let priorTranscript = (1...21).map { seq in
            GaryxTranscriptMessage(
                index: seq - 1,
                role: .user,
                kind: "user_input",
                text: "Existing turn \(seq)"
            )
        }
        let priorMessages = GaryxMobileTranscriptMapper.mobileMessages(from: priorTranscript)
        let priorSnapshot = GaryxRenderSnapshot(
            basedOnSeq: 371,
            rows: (1...21).map { seq in
                .userTurn(GaryxRenderUserTurnRow(
                    id: "user_turn:seq:\(seq)",
                    user: GaryxRenderMessageRef(id: "seq:\(seq)", seq: seq, role: "user"),
                    activity: []
                ))
            }
        )
        let optimistic = GaryxMobileMessage(
            id: "origin:\(origin)",
            role: .user,
            text: "A long mobile message that wraps across several composer lines before it is sent.",
            timestamp: nil,
            isStreaming: false,
            clientIntentId: origin,
            localState: .optimistic
        )
        let optimisticMessages = priorMessages + [optimistic]
        let optimisticRows = GaryxMobileRenderStateMapper.rows(
            snapshot: priorSnapshot,
            messages: optimisticMessages,
            transcriptMessages: priorTranscript
        )

        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 371, replayScope: .resume)
        let result = processor.processPayload(
            try fixture(named: "ios-send-jitter-frame.json"),
            threadId: "thread::send-jitter-capture"
        )
        guard case let .applyCommittedMessages(committed) = try XCTUnwrap(result.actions.first),
              case let .applyRenderSnapshot(committedSnapshot) = try XCTUnwrap(result.actions.last) else {
            return XCTFail("captured frame must apply its body before its render snapshot")
        }
        let committedTranscript = priorTranscript + committed
        let committedMessages = GaryxTranscriptMerge.mergedMessages(
            GaryxMobileTranscriptMapper.mobileMessages(from: committedTranscript),
            withLocal: optimisticMessages
        )
        let committedRows = GaryxMobileRenderStateMapper.rows(
            snapshot: committedSnapshot,
            messages: committedMessages,
            transcriptMessages: committedTranscript
        )

        let expectedTailID = "user_turn:origin:\(origin)"
        XCTAssertEqual(optimisticRows.count, 22)
        XCTAssertEqual(optimisticRows.last?.id, expectedTailID)
        XCTAssertEqual(optimisticRows.map(\.id), committedRows.map(\.id))
        XCTAssertEqual(
            committedRows.last?.userBlock?.message.id,
            "origin:\(origin)",
            "the committed body must reuse the optimistic message identity"
        )
        XCTAssertNotEqual(
            optimisticMessages,
            committedMessages,
            "the full-message SwiftUI observation fires when provenance becomes remoteFinal"
        )
        XCTAssertEqual(
            optimisticMessages.map(\.id),
            committedMessages.map(\.id),
            "the materialization changes content/provenance, not visible row identity"
        )
        let optimisticGeometry = optimisticMessages.map(GaryxMobileMessageGeometry.init)
        let committedGeometry = committedMessages.map(GaryxMobileMessageGeometry.init)
        XCTAssertEqual(
            optimisticGeometry,
            committedGeometry,
            "committing the captured origin must not change visible message geometry"
        )

        var scrollState = GaryxConversationScrollState()
        _ = scrollState.threadOpened()
        let appendRequest = scrollState.messagesChanged(
            previous: priorMessages.map(GaryxMobileMessageGeometry.init),
            current: optimisticGeometry,
            id: \.id,
            previousScopeIdentity: threadScope,
            currentScopeIdentity: threadScope,
            hasTailContent: true
        )
        XCTAssertEqual(appendRequest, .init(reason: .tailUpdate, animated: false))

        let materializationRequest = scrollState.messagesChanged(
            previous: optimisticGeometry,
            current: committedGeometry,
            id: \.id,
            previousScopeIdentity: threadScope,
            currentScopeIdentity: threadScope,
            hasTailContent: true
        )
        XCTAssertNil(
            materializationRequest,
            "FAILS ON BASELINE: stable committed materialization must not start another bottom-anchor retry chain"
        )
    }

    private func fixture(named name: String) throws -> String {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 {
            url.deleteLastPathComponent()
        }
        return try String(
            contentsOf: url
                .appendingPathComponent("test-fixtures")
                .appendingPathComponent("render-layer")
                .appendingPathComponent(name),
            encoding: .utf8
        )
    }
}
