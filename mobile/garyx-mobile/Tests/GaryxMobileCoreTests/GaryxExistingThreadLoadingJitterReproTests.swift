import Foundation
import XCTest
@testable import GaryxMobileCore

final class GaryxExistingThreadLoadingJitterReproTests: XCTestCase {
    /// `task-2610-markdown-table-frame.json` is an already-sanitized capture
    /// of canonical transcript seq 103...116 plus the matching idle
    /// `render_state` (`based_on_seq = 116`, `window.floor_seq = 103`,
    /// `tailActivity = none`). It gives this reproduction real historical
    /// message bodies and server-owned row identity without depending on UI.
    ///
    /// Sequence:
    /// 1. the existing-thread route reaches `.live` while history is still
    ///    loading (the presentation state intentionally does not await I/O);
    /// 2. the captured history arrives once and replaces the message loader;
    /// 3. the exact same snapshot/body input is reduced again: no server data,
    ///    row identity, row set, or visible geometry changes;
    /// 4. the initial-load scroll token confirms stable target placement and
    ///    settles without authorizing any delayed bottom-anchor writes.
    ///
    /// The final assertion is the headless equivalent of the reported
    /// post-loading jitter: stable rows must not keep mutating scroll position.
    func testUnchangedCapturedThreadStopsTailWritesAfterLoadingEnds() throws {
        let capture = try loadCapture()
        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 102, replayScope: .resume)
        let result = processor.processRenderFrame(capture, threadId: capture.threadId)

        XCTAssertNil(result.reconnect)
        guard case let .applyCommittedMessages(committed) = try XCTUnwrap(result.actions.first),
              case let .applyRenderSnapshot(snapshot) = try XCTUnwrap(result.actions.last) else {
            return XCTFail("captured frame must apply committed bodies before render_state")
        }
        XCTAssertEqual(snapshot.basedOnSeq, 116)
        XCTAssertEqual(snapshot.window, GaryxRenderWindow(floorSeq: 103, hasMoreAbove: true))
        XCTAssertEqual(snapshot.tailActivity, .none)

        let messages = GaryxMobileTranscriptMapper.mobileMessages(from: committed)
        let loadedRows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: messages,
            transcriptMessages: committed
        )
        let unchangedRows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: messages,
            transcriptMessages: committed
        )
        XCTAssertFalse(loadedRows.isEmpty, "the capture must represent an existing thread")
        XCTAssertEqual(loadedRows, unchangedRows)
        XCTAssertEqual(loadedRows.map(\.id), unchangedRows.map(\.id))

        var presentation = GaryxConversationRoutePresentationState()
        presentation.apply(lifecycle: .active)
        let frameInterval = 1.0 / 120.0
        for _ in 0..<GaryxConversationRoutePresentationState.defaultTerminalOpeningFrameCount {
            presentation.presentedFrame(interval: frameInterval)
        }
        for _ in 0..<GaryxConversationRoutePresentationState.defaultMaterializationFrameCount {
            presentation.presentedFrame(interval: frameInterval)
        }
        XCTAssertEqual(presentation.renderPhase, .live)
        XCTAssertTrue(presentation.allowsTranscriptInteraction)
        XCTAssertEqual(
            GaryxConversationTranscriptTreatmentPolicy.treatment(
                localRenderableRowCount: 0,
                hasRenderedSnapshot: false,
                isAwaitingInitialHistory: true
            ),
            .skeleton
        )
        XCTAssertEqual(
            GaryxConversationTranscriptTreatmentPolicy.treatment(
                localRenderableRowCount: loadedRows.count,
                hasRenderedSnapshot: true,
                isAwaitingInitialHistory: false
            ),
            .content,
            "the captured rows must replace the loading presentation"
        )

        let scope = "thread:\(capture.threadId)"
        let geometry = messages.map(GaryxMobileMessageGeometry.init)
        var scrollState = GaryxConversationScrollState()
        var scheduler = GaryxConversationScrollScheduler()

        let mountRequest = scrollState.threadOpened()
        let mountToken = scheduler.schedule(request: mountRequest)
        let loadingCompletionRequest = try XCTUnwrap(
            scrollState.messagesChanged(
                previous: [],
                current: geometry,
                id: \.id,
                previousScopeIdentity: scope,
                currentScopeIdentity: scope,
                hasTailContent: true
            )
        )
        XCTAssertEqual(loadingCompletionRequest.reason, .openingThread)
        let loadingCompletionToken = scheduler.schedule(
            request: loadingCompletionRequest
        )
        XCTAssertFalse(scheduler.isCurrent(mountToken))

        // The zero-delay attempt lands on the measured transcript tail. Its
        // geometry epoch includes the loaded capture before authorization, so
        // the unchanged frame below cannot justify a second position write.
        XCTAssertNil(
            scrollState.metricsChanged(
                GaryxConversationLayoutMetrics(
                    contentTopOffset: -2_000,
                    contentBottomOffset: 800,
                    viewportHeight: 800
                ),
                hasTailContent: true
            )
        )
        XCTAssertTrue(
            scheduler.authorizeAttempt(
                loadingCompletionToken,
                input: scrollState.scrollAttemptInput(
                    index: 0,
                    request: loadingCompletionRequest
                )
            )
        )

        let unchangedRequest = scrollState.messagesChanged(
            previous: geometry,
            current: geometry,
            id: \.id,
            previousScopeIdentity: scope,
            currentScopeIdentity: scope,
            hasTailContent: true
        )
        XCTAssertNil(unchangedRequest, "the equal server snapshot is a content no-op")
        XCTAssertNil(
            scrollState.renderRowsChanged(
                previousIds: loadedRows.map(\.id),
                currentIds: unchangedRows.map(\.id),
                previousScopeIdentity: scope,
                currentScopeIdentity: scope,
                hasTailContent: true
            ),
            "equal server row identities are not a prepend or replacement"
        )

        let postLoadingScrollWrites = loadingCompletionRequest.reason
            .retryDelayMilliseconds
            .enumerated()
            .compactMap { index, delay -> Int? in
                guard delay > 0,
                      scheduler.authorizeAttempt(
                          loadingCompletionToken,
                          input: scrollState.scrollAttemptInput(
                              index: index,
                              request: loadingCompletionRequest
                          )
                      )
                else {
                    return nil
                }
                return delay
            }

        XCTAssertEqual(
            postLoadingScrollWrites,
            [],
            """
            Equal captured rows with stable target placement must settle the \
            opening token without delayed scrollTo(bottom) writes.
            """
        )
        XCTAssertEqual(scheduler.lifecycle(of: loadingCompletionToken), .settled)
    }

    private func loadCapture() throws -> GaryxThreadRenderFrame {
        let url = try XCTUnwrap(
            Bundle.module.url(
                forResource: "task-2610-markdown-table-frame",
                withExtension: "json",
                subdirectory: "Fixtures"
            )
        )
        return try JSONDecoder().decode(
            GaryxThreadRenderFrame.self,
            from: Data(contentsOf: url)
        )
    }
}
