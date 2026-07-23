import Foundation
import XCTest
@testable import GaryxMobileCore

final class GaryxActiveThreadBottomReachabilityReproTests: XCTestCase {
    /// Sanitized from the reported thread at the screenshot boundary. The full
    /// server snapshot contained 11 rows at seq 274; the fixture retains its
    /// final task-notification turn, active assistant/tool step, tail activity,
    /// progress locus, and sparse committed bodies needed to resolve that row.
    ///
    /// The screenshot showed the active run at 2m24s while repeated swipes
    /// could not reach the live tail. This test combines that captured server
    /// state with the warm-reentry presentation path: opening pixels are legal,
    /// two opening callbacks establish a 120 Hz reference, and the mounted
    /// transcript then delivers at a stable 60 Hz. A live transcript must take
    /// interaction ownership; otherwise the opaque opening pixels continue to
    /// consume every scroll gesture above the composer.
    func testWarmReentryReleasesInteractionWhenCapturedActiveTailMountsAt60Hz() throws {
        let capture = try loadCapture()
        let frame = try JSONDecoder().decode(GaryxThreadRenderFrame.self, from: capture.data)
        let reproduction = capture.metadata.reproduction
        let snapshot = try XCTUnwrap(frame.renderState)

        XCTAssertEqual(snapshot.basedOnSeq, 274)
        XCTAssertEqual(reproduction.sourceRowCount, 11)
        XCTAssertEqual(snapshot.tailActivity, .toolActive)
        XCTAssertEqual(snapshot.activeToolGroupId, "tool_group:active-tail")
        XCTAssertEqual(snapshot.progressLocus, .toolGroup)
        XCTAssertEqual(reproduction.reportedWorkingElapsedSeconds, 144)

        let transcript = frame.events.compactMap { event -> GaryxTranscriptMessage? in
            guard var message = event.message, let seq = event.seq else { return nil }
            message.index = seq - 1
            return message
        }
        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: GaryxMobileTranscriptMapper.mobileMessages(from: transcript),
            transcriptMessages: transcript
        )

        XCTAssertEqual(rows.count, 1)
        let row = try XCTUnwrap(rows.first)
        let userMessage = try XCTUnwrap(row.userBlock?.message)
        guard case let .taskNotification(_, notification) =
            GaryxMobileMessagePresentation.make(for: userMessage)
        else {
            return XCTFail("the captured tail must retain its task-notification user row")
        }
        XCTAssertEqual(notification.taskId, "#TASK-1003")
        guard case let .turn(activeTurn) = try XCTUnwrap(row.activityRows.first) else {
            return XCTFail("the captured final activity must remain an agent turn")
        }
        XCTAssertTrue(activeTurn.isRunning)
        XCTAssertEqual(activeTurn.steps.count, 2)

        let treatment = GaryxConversationTranscriptTreatmentPolicy.treatment(
            localRenderableRowCount: rows.count,
            hasRenderedSnapshot: true,
            hasTranscriptSnapshotPixels: reproduction.warmReentryHasSnapshotPixels,
            isAwaitingInitialHistory: false
        )
        let input = GaryxConversationTranscriptPresentationInput(
            treatment: treatment,
            hasTranscriptSnapshotPixels: reproduction.warmReentryHasSnapshotPixels
        )
        var presentation = GaryxConversationRoutePresentationState()
        presentation.apply(lifecycle: .active)

        XCTAssertEqual(
            presentation.reconcileTranscriptPresentation(input),
            .openingCover(.snapshotPixels)
        )
        for interval in reproduction.openingFrameIntervals {
            presentation.presentedFrame(interval: interval)
        }
        XCTAssertEqual(presentation.renderPhase, .materializingConversation)
        XCTAssertFalse(presentation.allowsTranscriptInteraction)

        let requiredStableFrames = GaryxConversationRoutePresentationState
            .defaultMaterializationFrameCount
        for _ in 0..<(requiredStableFrames - 1) {
            presentation.presentedFrame(
                interval: reproduction.materializationFrameInterval
            )
        }
        XCTAssertFalse(
            presentation.allowsTranscriptInteraction,
            "the cover must remain until the complete stability window is delivered"
        )
        presentation.presentedFrame(
            interval: reproduction.materializationFrameInterval
        )

        XCTAssertTrue(
            presentation.allowsTranscriptInteraction,
            """
            A stable 60 Hz live transcript did not complete its normal \
            materialization window after a 120 Hz opening. The cached pixel \
            cover would keep consuming scroll gestures and make the active \
            tail unreachable behind the composer, as it did for the reported \
            \(reproduction.reportedWorkingElapsedSeconds) seconds.
            """
        )
    }

    private func loadCapture() throws -> (
        data: Data,
        metadata: Task2661CaptureMetadata
    ) {
        let url = try XCTUnwrap(
            Bundle.module.url(
                forResource: "task-2661-active-tail-frame",
                withExtension: "json",
                subdirectory: "Fixtures"
            )
        )
        let data = try Data(contentsOf: url)
        return (
            data,
            try JSONDecoder().decode(Task2661CaptureMetadata.self, from: data)
        )
    }
}

private struct Task2661CaptureMetadata: Decodable {
    let reproduction: Reproduction

    struct Reproduction: Decodable {
        let sourceRowCount: Int
        let reportedWorkingElapsedSeconds: Int
        let warmReentryHasSnapshotPixels: Bool
        let openingFrameIntervals: [TimeInterval?]
        let materializationFrameInterval: TimeInterval

        enum CodingKeys: String, CodingKey {
            case sourceRowCount = "source_row_count"
            case reportedWorkingElapsedSeconds = "reported_working_elapsed_seconds"
            case warmReentryHasSnapshotPixels = "warm_reentry_has_snapshot_pixels"
            case openingFrameIntervals = "opening_frame_intervals"
            case materializationFrameInterval = "materialization_frame_interval"
        }
    }
}
