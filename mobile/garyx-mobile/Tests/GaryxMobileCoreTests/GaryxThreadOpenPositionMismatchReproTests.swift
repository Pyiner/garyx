import Foundation
import XCTest
@testable import GaryxMobileCore

final class GaryxThreadOpenPositionMismatchReproTests: XCTestCase {
    /// Deterministic reproduction from #TASK-2697.
    ///
    /// The message shape comes from the sanitized production capture used by
    /// the existing loading-jitter regression. The geometry models the
    /// reachable warm-reentry case where the compositor snapshot is captured
    /// while the reader is browsing above the tail.
    func testWarmReentryPixelCoverAndLiveRevealHaveOneScrollPosition() throws {
        let frame = try loadCapture()
        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 102, replayScope: .resume)
        let result = processor.processRenderFrame(frame, threadId: frame.threadId)
        guard case let .applyCommittedMessages(committed) = try XCTUnwrap(result.actions.first),
              case let .applyRenderSnapshot(snapshot) = try XCTUnwrap(result.actions.last) else {
            return XCTFail("captured frame must materialize bodies and render_state")
        }
        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: GaryxMobileTranscriptMapper.mobileMessages(from: committed),
            transcriptMessages: committed
        )
        XCTAssertEqual(rows.count, 1)
        XCTAssertTrue(snapshot.window?.hasMoreAbove == true)

        let viewportHeight: CGFloat = 800
        let contentHeight: CGFloat = 3_200
        let capturedBrowsingOffset: CGFloat = 800
        let liveOpeningTailOffset = contentHeight - viewportHeight
        let viewportFrame = CGRect(x: 0, y: 120, width: 440, height: viewportHeight)

        var scroll = GaryxConversationScrollState()
        _ = scroll.userScrollInteractionChanged(isInteracting: true)
        _ = scroll.metricsChanged(
            GaryxConversationLayoutMetrics(
                contentTopOffset: -capturedBrowsingOffset,
                contentBottomOffset: contentHeight - capturedBrowsingOffset,
                viewportHeight: viewportHeight
            ),
            hasTailContent: true
        )
        XCTAssertFalse(scroll.isFollowingTail, "the captured viewport is browsing history")
        _ = scroll.userScrollInteractionChanged(isInteracting: false)
        let openingRequest = scroll.threadOpened()
        XCTAssertEqual(openingRequest.reason, .openingThread)
        XCTAssertTrue(scroll.isFollowingTail, "live mount discards the captured reading position")

        var capture = GaryxConversationOpeningViewportCaptureState(
            requiredStableSampleCount: 1
        )
        let browsingSample = openingSample(
            viewportFrame: viewportFrame,
            contentHeight: contentHeight,
            contentOffsetY: capturedBrowsingOffset,
            isFollowingTail: false
        )
        XCTAssertNil(
            capture.observe(browsingSample),
            "the 800pt browsing snapshot must never enter the opening-cover cache"
        )

        let tailSample = openingSample(
            viewportFrame: viewportFrame,
            contentHeight: contentHeight,
            contentOffsetY: liveOpeningTailOffset,
            isFollowingTail: true
        )
        let contract = try XCTUnwrap(capture.observe(tailSample))
        XCTAssertTrue(
            GaryxConversationOpeningViewportContractPolicy.canServe(
                contract,
                revisionMatches: true,
                visibleViewportFrameInPage: tailSample.visibleViewportFrameInPage
            )
        )

        let input = GaryxConversationTranscriptPresentationInput(
            treatment: .content,
            openingViewportContractID: "task-2697-tail-contract"
        )
        XCTAssertEqual(
            GaryxConversationTranscriptPresentationPolicy.presentation(
                renderPhase: .openingPage,
                input: input
            ),
            .openingCover(.snapshotPixels)
        )
        var route = GaryxConversationRoutePresentationState()
        route.apply(lifecycle: .active)
        let frameInterval = 1.0 / 120.0
        for _ in 0..<GaryxConversationRoutePresentationState.defaultTerminalOpeningFrameCount {
            route.presentedFrame(interval: frameInterval)
        }
        for _ in 0..<GaryxConversationRoutePresentationState.defaultMaterializationFrameCount {
            route.presentedFrame(
                interval: frameInterval,
                openingViewportReadiness: .pending
            )
        }
        XCTAssertEqual(
            route.renderPhase,
            .materializingConversation,
            "frame cadence alone cannot reveal a pixel cover"
        )
        let readiness =
            GaryxConversationOpeningViewportContractPolicy.revealReadiness(
                for: contract,
                live: tailSample,
                revisionMatches: true
            )
        XCTAssertEqual(readiness, .matched)
        route.presentedFrame(
            interval: frameInterval,
            openingViewportReadiness: readiness
        )
        XCTAssertEqual(route.renderPhase, .live)

        XCTAssertEqual(
            contract.captureGeometry.contentOffset.y,
            liveOpeningTailOffset,
            "the only serviceable cover and the live opening resolve 2400"
        )
    }

    private func openingSample(
        viewportFrame: CGRect,
        contentHeight: CGFloat,
        contentOffsetY: CGFloat,
        isFollowingTail: Bool
    ) -> GaryxConversationOpeningViewportSample {
        GaryxConversationOpeningViewportSample(
            captureGeometry: GaryxConversationTranscriptSnapshotCaptureGeometry(
                viewportFrameInPage: viewportFrame,
                adjustedContentInsets: .init(top: 0, left: 0, bottom: 0, right: 0),
                contentOffset: CGPoint(x: 0, y: contentOffsetY)
            ),
            visibleViewportFrameInPage: viewportFrame,
            contentSize: CGSize(width: viewportFrame.width, height: contentHeight),
            displayScale: 3,
            layoutEpoch: 7,
            isFollowingTail: isFollowingTail,
            isUserInteracting: false
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
