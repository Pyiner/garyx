import Foundation
import XCTest
@testable import GaryxMobileCore

final class GaryxSendAnchoredTranscriptReproTests: XCTestCase {
    private struct Capture: Decodable {
        struct OptimisticMessage: Decodable {
            let text: String
        }

        let captureKind: String
        let sourceDevice: String
        let note: String
        let threadId: String
        let originId: String
        let baselineMessages: [GaryxTranscriptMessage]
        let optimisticMessage: OptimisticMessage
        let frames: [GaryxThreadRenderFrame]

        enum CodingKeys: String, CodingKey {
            case captureKind = "capture_kind"
            case sourceDevice = "source_device"
            case note
            case threadId = "thread_id"
            case originId = "origin_id"
            case baselineMessages = "baseline_messages"
            case optimisticMessage = "optimistic_message"
            case frames
        }
    }

    /// Repro-first gate from a sanitized, real iPhone 17 Pro Max / iOS 26.5
    /// simulator SSE capture. The send was made from the production composer
    /// into an existing Codex thread. The provider emitted run-start,
    /// origin-bearing user materialization, assistant content while the run
    /// remained active, done, and run-complete frames.
    ///
    /// This regression gate pins both candidate jitter paths:
    /// - mapper/merge row identity is stable and materializes exactly once;
    /// - one local send creates one top-anchor request, while every captured
    ///   ACK/thinking/assistant/final change is scroll-silent.
    func testCapturedExistingThreadSequenceAnchorsOnceWithoutIdentityDrift() throws {
        let capture = try loadCapture()
        XCTAssertEqual(capture.captureKind, "sanitized_live_ios_simulator_sse")
        XCTAssertEqual(capture.sourceDevice, "iPhone 17 Pro Max / iOS 26.5 / light mode")
        XCTAssertTrue(capture.note.contains("No token-partial render frame was emitted"))
        XCTAssertEqual(capture.frames.map { $0.renderState?.basedOnSeq }, [7, 8, 9, 10, 11])

        let optimistic = GaryxMobileMessage(
            id: "origin:\(capture.originId)",
            role: .user,
            text: capture.optimisticMessage.text,
            timestamp: nil,
            isStreaming: false,
            clientIntentId: capture.originId,
            localState: .optimistic
        )
        var transcriptByIndex = Dictionary(
            uniqueKeysWithValues: capture.baselineMessages.compactMap { message in
                message.index.map { ($0, message) }
            }
        )
        var localMessages =
            GaryxMobileTranscriptMapper.mobileMessages(from: capture.baselineMessages)
            + [optimistic]
        let baselineGeometry = GaryxMobileTranscriptMapper
            .mobileMessages(from: capture.baselineMessages)
            .map(GaryxMobileMessageGeometry.init)

        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 6, replayScope: .resume)
        var rowIdFrames = [[String]]()
        var optimisticMaterializationCount = 0
        var capturedGeometryBeforeMaterialization: GaryxMobileMessageGeometry?
        var capturedGeometryAfterMaterialization: GaryxMobileMessageGeometry?
        var lastGeometry = localMessages.map(GaryxMobileMessageGeometry.init)
        var thinkingPresentation = GaryxTailThinkingPresentationState()
        let optimisticRenderInput = GaryxConversationRouteRenderInputResolver.resolve(
            destination: .conversation(threadID: capture.threadId),
            draftMessages: [],
            threadMessages: localMessages,
            threadSnapshot: nil,
            threadTranscriptMessages: capture.baselineMessages
        )
        XCTAssertEqual(optimisticRenderInput.tailThinkingPresentationMode, .immediate)
        XCTAssertTrue(
            thinkingPresentation.update(
                mode: optimisticRenderInput.tailThinkingPresentationMode,
                now: 0
            ),
            "the captured local row and thinking label present together"
        )
        var thinkingModes = [optimisticRenderInput.tailThinkingPresentationMode]

        var scrollState = GaryxConversationScrollState()
        _ = scrollState.threadOpened()
        let anchorRowId = "user_turn:\(optimistic.id)"
        let scrollRequests = [
            scrollState.localSendPresented(anchorRowId: anchorRowId)
        ]
        XCTAssertNil(
            scrollState.messagesChanged(
                previous: baselineGeometry,
                current: lastGeometry,
                id: \.id,
                previousScopeIdentity: capture.threadId,
                currentScopeIdentity: capture.threadId,
                hasTailContent: true
            ),
            "the optimistic append is silent after the same-frame local-send event"
        )

        var thinkingWasVisible = false
        for frame in capture.frames {
            let result = processor.processRenderFrame(frame, threadId: capture.threadId)
            XCTAssertNil(result.reconnect)

            var snapshot: GaryxRenderSnapshot?
            for action in result.actions {
                switch action {
                case .applyCommittedMessages(let committed):
                    for message in committed {
                        if let index = message.index {
                            transcriptByIndex[index] = message
                        }
                    }
                case .applyRenderSnapshot(let applied):
                    snapshot = applied
                case .resetCommittedCacheBelow, .refetchAfterControlRewrite, .fallback:
                    XCTFail("captured contiguous live frame must not reset or fall back")
                }
            }
            let appliedSnapshot = try XCTUnwrap(snapshot)
            let transcript = transcriptByIndex
                .sorted { $0.key < $1.key }
                .map(\.value)
            let incoming = GaryxMobileTranscriptMapper.mobileMessages(from: transcript)
            let previousOriginState = localMessages.first { $0.id == optimistic.id }?.localState
            localMessages = GaryxTranscriptMerge.mergedMessages(
                incoming,
                withLocal: localMessages
            )
            let currentOrigin = try XCTUnwrap(
                localMessages.first { $0.id == optimistic.id }
            )
            if previousOriginState == .optimistic,
               currentOrigin.localState == .remoteFinal {
                optimisticMaterializationCount += 1
                capturedGeometryAfterMaterialization = GaryxMobileMessageGeometry(
                    message: currentOrigin
                )
            }
            if currentOrigin.localState == .optimistic {
                capturedGeometryBeforeMaterialization = GaryxMobileMessageGeometry(
                    message: currentOrigin
                )
            }

            let rows = GaryxMobileRenderStateMapper.rows(
                snapshot: appliedSnapshot,
                messages: localMessages,
                transcriptMessages: transcript
            )
            rowIdFrames.append(rows.map(\.id))
            let renderInput = GaryxConversationRouteRenderInputResolver.resolve(
                destination: .conversation(threadID: capture.threadId),
                draftMessages: [],
                threadMessages: localMessages,
                threadSnapshot: appliedSnapshot,
                threadTranscriptMessages: transcript
            )
            thinkingModes.append(renderInput.tailThinkingPresentationMode)
            let thinkingVisible = thinkingPresentation.update(
                mode: renderInput.tailThinkingPresentationMode,
                now: TimeInterval(thinkingModes.count) / 100
            )
            if renderInput.tailThinkingPresentationMode != .hidden {
                XCTAssertTrue(
                    thinkingVisible,
                    "ACK must hand off the already-visible label without a debounce gap"
                )
            }

            let nextGeometry = localMessages.map(GaryxMobileMessageGeometry.init)
            XCTAssertNil(
                scrollState.messagesChanged(
                    previous: lastGeometry,
                    current: nextGeometry,
                    id: \.id,
                    previousScopeIdentity: capture.threadId,
                    currentScopeIdentity: capture.threadId,
                    hasTailContent: true
                ),
                "ACK and streamed body changes must not move a send-anchored transcript"
            )
            lastGeometry = nextGeometry

            let thinkingIsVisible = appliedSnapshot.tailActivity == .thinking
            if thinkingIsVisible && !thinkingWasVisible {
                XCTAssertNil(
                    scrollState.thinkingIndicatorShown(),
                    "the captured thinking frame must not start another scroll chain"
                )
            }
            thinkingWasVisible = thinkingIsVisible

            var metrics = scrollState.metrics
            metrics.contentTopOffset = -1_200
            metrics.contentBottomOffset += 80
            metrics.contentTailOffset = metrics.contentBottomOffset
            metrics.viewportHeight = 800
            XCTAssertNil(
                scrollState.metricsChanged(metrics, hasTailContent: true),
                "streamed layout measurement must stay scroll-silent while anchored"
            )
        }

        let expectedRows = [
            "user_turn:seq:2",
            anchorRowId
        ]
        XCTAssertFalse(rowIdFrames.isEmpty)
        XCTAssertTrue(rowIdFrames.allSatisfy { $0 == expectedRows })
        for (previous, current) in zip(rowIdFrames, rowIdFrames.dropFirst()) {
            XCTAssertEqual(Array(current.prefix(previous.count)), previous)
        }
        XCTAssertEqual(optimisticMaterializationCount, 1)
        XCTAssertEqual(
            capturedGeometryBeforeMaterialization,
            capturedGeometryAfterMaterialization,
            "ACK materialization must preserve visible row geometry"
        )
        XCTAssertEqual(localMessages.first { $0.id == optimistic.id }?.localState, .remoteFinal)
        XCTAssertEqual(
            thinkingModes,
            [.immediate, .immediate, .debounced, .debounced, .debounced, .hidden]
        )
        XCTAssertFalse(thinkingPresentation.isVisible)

        XCTAssertEqual(scrollRequests.count, 1)
        XCTAssertEqual(
            scrollRequests.first,
            .init(
                reason: .localSend,
                target: .row(id: anchorRowId),
                alignment: .top,
                animated: true
            )
        )
        XCTAssertEqual(scrollState.anchoring, .sendAnchored(anchorRowId: anchorRowId))

        var scheduler = GaryxConversationScrollScheduler()
        let request = try XCTUnwrap(scrollRequests.first)
        let token = scheduler.schedule(request: request).token
        var authorizedWriteCount = 0
        if scheduler.authorizeAttempt(
            token,
            input: scrollState.scrollAttemptInput(
                index: 0,
                request: request,
                rowTargetViewportOffset: 320
            )
        ) {
            authorizedWriteCount += 1
        }
        if scheduler.authorizeAttempt(
            token,
            input: scrollState.scrollAttemptInput(
                index: 1,
                request: request,
                rowTargetViewportOffset: 0
            )
        ) {
            authorizedWriteCount += 1
        }
        XCTAssertEqual(
            authorizedWriteCount,
            1,
            "one settled local-send request performs one position write"
        )
        XCTAssertEqual(scheduler.lifecycle(of: token), .settled)
    }

    private func loadCapture() throws -> Capture {
        let url = try XCTUnwrap(
            Bundle.module.url(
                forResource: "task-2680-send-anchor-live-sequence",
                withExtension: "json",
                subdirectory: "Fixtures"
            )
        )
        return try JSONDecoder().decode(Capture.self, from: Data(contentsOf: url))
    }
}
