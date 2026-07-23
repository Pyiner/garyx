import Foundation
import XCTest
@testable import GaryxMobileCore

final class GaryxExistingThreadLoadingJitterReproTests: XCTestCase {
    /// EXPECTED FAILURE (#TASK-2630).
    ///
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
    /// 4. the initial-load scroll token nevertheless authorizes six delayed
    ///    bottom-anchor writes after loading ended.
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
            GaryxConversationOpeningTranscriptPolicy.presentation(localRenderableRowCount: 0),
            .loading
        )
        XCTAssertEqual(
            GaryxConversationOpeningTranscriptPolicy.presentation(
                localRenderableRowCount: loadedRows.count
            ),
            .localMessages,
            "the captured rows must replace the loading presentation"
        )

        let scope = "thread:\(capture.threadId)"
        let geometry = messages.map(GaryxMobileMessageGeometry.init)
        var scrollState = GaryxConversationScrollState()
        var scheduler = GaryxConversationTailScrollScheduler()

        let mountRequest = scrollState.threadOpened()
        let mountToken = scheduler.schedule(reason: mountRequest.reason)
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
            reason: loadingCompletionRequest.reason
        )
        XCTAssertFalse(scheduler.isCurrent(mountToken))

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
                      scheduler.isCurrent(loadingCompletionToken),
                      scrollState.shouldRunTailScrollAttempt(
                          index: index,
                          reason: loadingCompletionRequest.reason
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
            EXPECTED FAILURE (#TASK-2630): equal captured rows still authorize \
            delayed scrollTo(bottom) writes after the loading row disappears
            """
        )
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
