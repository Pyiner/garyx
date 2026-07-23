import Foundation
import XCTest
@testable import GaryxMobileCore

final class GaryxExistingThreadLoadingFlashReproTests: XCTestCase {
    /// Regression for #TASK-2644. The production Core composition must derive
    /// the treatment again when persisted rows restore, then resolve directly
    /// to live content instead of retaining a stale skeleton cover.
    func testPersistedRowsDoNotMixWithLoadingSkeletonBeforeLiveReveal() throws {
        let capturedTranscript = try capturedPersistedTranscript()
        let restoredMessages = GaryxMobileTranscriptMapper.mobileMessages(
            from: capturedTranscript.messages
        )
        let restoredRows = GaryxMobileRenderStateMapper.rows(
            snapshot: capturedTranscript.renderSnapshot,
            messages: restoredMessages,
            transcriptMessages: capturedTranscript.messages
        )
        let repeatedRows = GaryxMobileRenderStateMapper.rows(
            snapshot: capturedTranscript.renderSnapshot,
            messages: restoredMessages,
            transcriptMessages: capturedTranscript.messages
        )

        XCTAssertFalse(
            restoredRows.isEmpty,
            "the captured existing thread must have locally persisted renderable rows"
        )
        XCTAssertEqual(restoredRows, repeatedRows)
        XCTAssertEqual(restoredRows.map(\.id), repeatedRows.map(\.id))
        XCTAssertFalse(
            visibleMessages(in: restoredRows).contains {
                GaryxMobileMessagePresentation.make(for: $0) == .historySkeleton
            },
            """
            the fully resolved captured render_state contains no row-level \
            placeholder; the reproduced skeleton comes from the separate \
            opening-cover surface
            """
        )

        var route = GaryxConversationRoutePresentationState()
        route.apply(lifecycle: .active)
        let emptyInput = presentationInput(
            localRenderableRowCount: 0,
            hasRenderedSnapshot: false,
            isAwaitingInitialHistory: true
        )

        let openingFrame = visibleFrame(
            route: route,
            input: emptyInput,
            localRows: [],
            hasRenderedSnapshot: false
        )

        // The first delivered frame starts preparation; the second mounts the
        // live transcript behind the one opaque skeleton cover.
        route.presentedFrame(interval: nil)
        route.presentedFrame(interval: 1.0 / 120.0)
        XCTAssertEqual(route.renderPhase, .materializingConversation)
        let materializingFrame = visibleFrame(
            route: route,
            input: emptyInput,
            localRows: [],
            hasRenderedSnapshot: false
        )

        // Disk restore is a live input change. Content without snapshot pixels
        // makes the old cover illegal in the same frame, before the driver's
        // state publication catches up.
        let restoredInput = presentationInput(
            localRenderableRowCount: restoredRows.count,
            hasRenderedSnapshot: capturedTranscript.renderSnapshot != nil,
            isAwaitingInitialHistory: true
        )
        let restoreLandedFrame = visibleFrame(
            route: route,
            input: restoredInput,
            localRows: restoredRows,
            hasRenderedSnapshot: capturedTranscript.renderSnapshot != nil
        )
        XCTAssertEqual(
            route.reconcileTranscriptPresentation(restoredInput),
            .live(.content)
        )
        XCTAssertEqual(route.renderPhase, .live)
        let liveFrame = visibleFrame(
            route: route,
            input: restoredInput,
            localRows: restoredRows,
            hasRenderedSnapshot: capturedTranscript.renderSnapshot != nil
        )

        let observed = [
            openingFrame,
            materializingFrame,
            restoreLandedFrame,
            liveFrame,
        ]
        XCTAssertEqual(
            observed.map(\.contentKinds),
            [
                [.skeleton],
                [.skeleton],
                [.content],
                [.content],
            ],
            "every frame has one live-derived treatment"
        )
        XCTAssertEqual(
            observed.filter(\.mixesSkeletonAndContent),
            [],
            "skeleton and restored content must never share a visible frame"
        )
    }

    /// Drives the complete captured cold-open sequence frame by frame:
    /// empty memory -> delivered opening frames -> materialization -> async
    /// restore -> immediate live promotion. Every recorded frame asserts the
    /// four normative invariants from the approved design.
    func testCapturedColdOpenCompositionSequenceMaintainsINV1ThroughINV4() throws {
        let capturedTranscript = try capturedPersistedTranscript()
        let restoredMessages = GaryxMobileTranscriptMapper.mobileMessages(
            from: capturedTranscript.messages
        )
        let restoredRows = GaryxMobileRenderStateMapper.rows(
            snapshot: capturedTranscript.renderSnapshot,
            messages: restoredMessages,
            transcriptMessages: capturedTranscript.messages
        )
        XCTAssertFalse(restoredRows.isEmpty)

        let emptyInput = presentationInput(
            localRenderableRowCount: 0,
            hasRenderedSnapshot: false,
            isAwaitingInitialHistory: true
        )
        let restoredInput = presentationInput(
            localRenderableRowCount: restoredRows.count,
            hasRenderedSnapshot: capturedTranscript.renderSnapshot != nil,
            isAwaitingInitialHistory: true
        )
        var route = GaryxConversationRoutePresentationState()
        route.apply(lifecycle: .active)
        var frames: [VisibleFrame] = [
            visibleFrame(
                route: route,
                input: emptyInput,
                localRows: [],
                hasRenderedSnapshot: false
            ),
        ]

        for _ in 0..<GaryxConversationRoutePresentationState.defaultTerminalOpeningFrameCount {
            route.presentedFrame(interval: 1.0 / 120.0)
            frames.append(
                visibleFrame(
                    route: route,
                    input: emptyInput,
                    localRows: [],
                    hasRenderedSnapshot: false
                )
            )
        }
        XCTAssertEqual(route.renderPhase, .materializingConversation)

        // Exercise several delivered materialization frames before the async
        // restore lands; none may expose the mounted live graph through the
        // opaque skeleton cover.
        for _ in 0..<3 {
            route.presentedFrame(interval: 1.0 / 120.0)
            frames.append(
                visibleFrame(
                    route: route,
                    input: emptyInput,
                    localRows: [],
                    hasRenderedSnapshot: false
                )
            )
        }

        frames.append(
            visibleFrame(
                route: route,
                input: restoredInput,
                localRows: restoredRows,
                hasRenderedSnapshot: true
            )
        )
        _ = route.reconcileTranscriptPresentation(restoredInput)
        frames.append(
            visibleFrame(
                route: route,
                input: restoredInput,
                localRows: restoredRows,
                hasRenderedSnapshot: true
            )
        )

        for frame in frames {
            // INV-1: one and only one visible treatment per frame.
            XCTAssertEqual(frame.contentKinds.count, 1, "\(frame)")
            XCTAssertFalse(frame.mixesSkeletonAndContent, "\(frame)")

            // INV-2: rows or a rendered snapshot force content continuously.
            if frame.localRenderableRowCount > 0 || frame.hasRenderedSnapshot {
                XCTAssertEqual(frame.contentKinds, [.content], "\(frame)")
            }

            // INV-3: cover and live share the exact treatment input; every
            // retained cover satisfies the one Core legality policy.
            XCTAssertEqual(frame.presentation.treatment, frame.input.treatment, "\(frame)")
            if frame.presentation.showsOpeningCover {
                XCTAssertTrue(
                    GaryxConversationTranscriptPresentationPolicy.coverIsLegal(
                        for: frame.input
                    ),
                    "\(frame)"
                )
            }
        }

        // INV-4: the only treatment edge is one direct skeleton -> content
        // replacement. The Core presentation algebra has no opacity state.
        let treatmentEdges = zip(frames, frames.dropFirst()).compactMap {
            $0.contentKinds == $1.contentKinds
                ? nil
                : ($0.contentKinds, $1.contentKinds)
        }
        XCTAssertEqual(treatmentEdges.count, 1)
        XCTAssertEqual(treatmentEdges.first?.0, [.skeleton])
        XCTAssertEqual(treatmentEdges.first?.1, [.content])
        XCTAssertEqual(route.renderPhase, .live)
        XCTAssertEqual(frames.last?.presentation, .live(.content))
    }

    private func capturedPersistedTranscript() throws -> GaryxCachedTranscript {
        let url = try XCTUnwrap(
            Bundle.module.url(
                forResource: "task-2610-markdown-table-frame",
                withExtension: "json",
                subdirectory: "Fixtures"
            )
        )
        let frame = try JSONDecoder().decode(
            GaryxThreadRenderFrame.self,
            from: Data(contentsOf: url)
        )

        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 102, replayScope: .resume)
        let result = processor.processRenderFrame(frame, threadId: frame.threadId)
        guard case let .applyCommittedMessages(messages) = try XCTUnwrap(result.actions.first),
              case let .applyRenderSnapshot(snapshot) = try XCTUnwrap(result.actions.last) else {
            throw ReproductionError.captureDidNotContainCommittedRowsAndSnapshot
        }

        let persisted = GaryxCachedTranscript(
            threadId: frame.threadId,
            savedAt: Date(timeIntervalSince1970: 0),
            messages: messages,
            renderSnapshot: snapshot,
            hasMoreBefore: true,
            nextBeforeIndex: 102
        )
        let encoded = try JSONEncoder().encode(persisted)
        return try JSONDecoder().decode(GaryxCachedTranscript.self, from: encoded)
    }

    private func presentationInput(
        localRenderableRowCount: Int,
        hasRenderedSnapshot: Bool,
        isAwaitingInitialHistory: Bool,
        hasTranscriptSnapshotPixels: Bool = false
    ) -> GaryxConversationTranscriptPresentationInput {
        GaryxConversationTranscriptPresentationInput(
            treatment: GaryxConversationTranscriptTreatmentPolicy.treatment(
                localRenderableRowCount: localRenderableRowCount,
                hasRenderedSnapshot: hasRenderedSnapshot,
                hasTranscriptSnapshotPixels: hasTranscriptSnapshotPixels,
                isAwaitingInitialHistory: isAwaitingInitialHistory
            ),
            hasTranscriptSnapshotPixels: hasTranscriptSnapshotPixels
        )
    }

    /// Observes the Core-owned production composition directly. There is no
    /// test-only adapter mirroring independent SwiftUI visibility gates.
    private func visibleFrame(
        route: GaryxConversationRoutePresentationState,
        input: GaryxConversationTranscriptPresentationInput,
        localRows: [GaryxMobileTurnRow],
        hasRenderedSnapshot: Bool
    ) -> VisibleFrame {
        let presentation = GaryxConversationTranscriptPresentationPolicy.presentation(
            renderPhase: route.renderPhase,
            input: input
        )
        return VisibleFrame(
            phase: route.renderPhase,
            presentation: presentation,
            input: input,
            contentKinds: [presentation.treatment],
            localRenderableRowCount: localRows.count,
            hasRenderedSnapshot: hasRenderedSnapshot,
            rowIDs: localRows.map(\.id)
        )
    }

    private func visibleMessages(
        in rows: [GaryxMobileTurnRow]
    ) -> [GaryxMobileMessage] {
        rows.flatMap { row in
            var messages = row.userBlock.map { [$0.message] } ?? []
            for activityRow in row.activityRows {
                switch activityRow {
                case .flat(let block):
                    messages.append(block.message)
                case .turn(let turn):
                    messages += turn.steps.map(\.message)
                    if let finalBlock = turn.finalBlock {
                        messages.append(finalBlock.message)
                    }
                }
            }
            return messages
        }
    }
}

private extension GaryxExistingThreadLoadingFlashReproTests {
    enum ReproductionError: Error {
        case captureDidNotContainCommittedRowsAndSnapshot
    }

    struct VisibleFrame: Equatable, CustomStringConvertible {
        let phase: GaryxConversationRouteRenderPhase
        let presentation: GaryxConversationTranscriptPresentation
        let input: GaryxConversationTranscriptPresentationInput
        let contentKinds: [GaryxConversationTranscriptTreatment]
        let localRenderableRowCount: Int
        let hasRenderedSnapshot: Bool
        let rowIDs: [String]

        var mixesSkeletonAndContent: Bool {
            contentKinds.contains(.skeleton) && contentKinds.contains(.content)
        }

        var description: String {
            "\(phase.rawValue):\(presentation):rows=\(rowIDs)"
        }
    }
}
