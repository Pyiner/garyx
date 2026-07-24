import Foundation
import XCTest
@testable import GaryxMobileCore

final class GaryxNewThreadFirstSendReproTests: XCTestCase {
    /// Sanitized production frame captured from the iOS send path. The test
    /// reduces its origin-bearing committed user turn to the empty-thread case
    /// and drives the same route-input resolver used by the mounted app.
    func testCapturedFirstSendRendersBeforeCommittedFrameAndConvergesExactlyOnce() throws {
        let payload = try fixture(named: "ios-send-jitter-frame.json")
        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 371, replayScope: .resume)
        let result = processor.processPayload(
            payload,
            threadId: "thread::send-jitter-capture"
        )
        guard case let .applyCommittedMessages(committedMessages) = try XCTUnwrap(result.actions.first),
              case let .applyRenderSnapshot(capturedSnapshot) = try XCTUnwrap(result.actions.last),
              let committedMessage = committedMessages.last,
              var optimisticMessage = GaryxMobileTranscriptMapper.mobileMessages(
                  from: [committedMessage]
              ).last,
              let committedRow = capturedSnapshot.rows.last
        else {
            return XCTFail("captured frame must contain the origin-bearing user materialization")
        }

        optimisticMessage.localState = .optimistic
        optimisticMessage.historyIndex = nil
        optimisticMessage.timestamp = nil
        let expectedMessageID = optimisticMessage.id
        let expectedRowID = "user_turn:\(expectedMessageID)"

        let beforeCommit = GaryxConversationRouteRenderInputResolver.resolve(
            destination: .conversationDraft(draftID: "new-thread"),
            draftMessages: [optimisticMessage],
            threadMessages: [],
            threadSnapshot: nil,
            threadTranscriptMessages: []
        )
        let beforeCommitRows = GaryxMobileRenderStateMapper.rows(
            snapshot: beforeCommit.snapshot,
            messages: beforeCommit.messages,
            transcriptMessages: beforeCommit.transcriptMessages
        )

        XCTAssertEqual(
            beforeCommitRows.map(\.id),
            [expectedRowID],
            "FAILS ON BASELINE: a new-thread send must render its local user row before any committed frame"
        )
        XCTAssertTrue(beforeCommit.showsPendingAcknowledgement)
        XCTAssertTrue(beforeCommit.showsTailThinking)

        let promoted = GaryxConversationRouteRenderInputResolver.resolve(
            destination: .conversation(threadID: "thread::send-jitter-capture"),
            draftMessages: [optimisticMessage],
            threadMessages: [optimisticMessage],
            threadSnapshot: nil,
            threadTranscriptMessages: []
        )
        let promotedRows = GaryxMobileRenderStateMapper.rows(
            snapshot: promoted.snapshot,
            messages: promoted.messages,
            transcriptMessages: promoted.transcriptMessages
        )
        XCTAssertEqual(promotedRows.map(\.id), [expectedRowID])
        XCTAssertTrue(promoted.showsPendingAcknowledgement)
        XCTAssertTrue(promoted.showsTailThinking)

        let firstThreadSnapshot = GaryxRenderSnapshot(
            basedOnSeq: capturedSnapshot.basedOnSeq,
            rows: [committedRow],
            tailActivity: capturedSnapshot.tailActivity,
            activeToolGroupId: capturedSnapshot.activeToolGroupId,
            progressLocus: capturedSnapshot.progressLocus,
            filteredPlaceholders: capturedSnapshot.filteredPlaceholders,
            rateLimit: capturedSnapshot.rateLimit,
            window: capturedSnapshot.window,
            rowsHash: capturedSnapshot.rowsHash
        )
        let committedMobile = GaryxMobileTranscriptMapper.mobileMessages(
            from: [committedMessage]
        )
        let reconciledMessages = GaryxTranscriptMerge.mergedMessages(
            committedMobile,
            withLocal: [optimisticMessage]
        )
        let afterCommit = GaryxConversationRouteRenderInputResolver.resolve(
            destination: .conversation(threadID: "thread::send-jitter-capture"),
            draftMessages: [optimisticMessage],
            threadMessages: reconciledMessages,
            threadSnapshot: firstThreadSnapshot,
            threadTranscriptMessages: [committedMessage]
        )
        let afterCommitRows = GaryxMobileRenderStateMapper.rows(
            snapshot: afterCommit.snapshot,
            messages: afterCommit.messages,
            transcriptMessages: afterCommit.transcriptMessages
        )

        XCTAssertEqual(afterCommitRows.map(\.id), [expectedRowID])
        XCTAssertFalse(afterCommit.showsPendingAcknowledgement)
        XCTAssertTrue(
            afterCommit.showsTailThinking,
            "the captured committed frame owns the continuing thinking state"
        )
        XCTAssertEqual(
            afterCommitRows.filter { $0.userBlock?.message.id == expectedMessageID }.count,
            1,
            "the committed origin must replace the optimistic row exactly once"
        )

        let settled = GaryxConversationRouteRenderInputResolver.resolve(
            destination: .conversation(threadID: "thread::send-jitter-capture"),
            draftMessages: [optimisticMessage],
            threadMessages: reconciledMessages,
            threadSnapshot: GaryxRenderSnapshot(
                basedOnSeq: capturedSnapshot.basedOnSeq,
                rows: [committedRow]
            ),
            threadTranscriptMessages: [committedMessage]
        )
        XCTAssertFalse(
            settled.showsTailThinking,
            "committed provenance plus an idle server snapshot must settle all pending chrome"
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
