import Foundation
import XCTest
@testable import GaryxMobileCore

final class GaryxExistingThreadLoadingFlashReproTests: XCTestCase {
    /// EXPECTED FAILURE (#TASK-2644).
    ///
    /// The fixture is the repository's sanitized capture of one committed
    /// historical turn and its matching server-owned `render_state`. The test
    /// round-trips that capture through `GaryxCachedTranscript` to establish
    /// that the thread has locally persisted, renderable rows before it opens.
    ///
    /// Production then has two independent opening inputs:
    ///
    /// - route metadata only sees the still-empty in-memory row store and
    ///   freezes its opening treatment as `.loading`;
    /// - the staged route mounts the restored live transcript during
    ///   `.materializingConversation` while its opening cover remains visible.
    ///
    /// `GaryxConversationView` places those two surfaces in the same ZStack.
    /// The loading cover has a transparent background, so the materialization
    /// frame visibly contains both the skeleton and the real row. Advancing to
    /// `.live` removes the skeleton compositor layer, producing the reported
    /// loading-completion flash.
    func testPersistedRowsMixWithLoadingSkeletonBeforeLiveReveal() throws {
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

        // `cacheConversationOpeningMetadata` runs before the asynchronous disk
        // restore. Its only Core input is the empty in-memory row count; the
        // locally persisted rows above are not represented in that decision.
        let frozenOpeningPresentation =
            GaryxConversationOpeningTranscriptPolicy.presentation(
                localRenderableRowCount: 0,
                hasRenderedSnapshot: false
            )

        var route = GaryxConversationRoutePresentationState()
        route.apply(lifecycle: .active)

        let openingFrame = visibleFrame(
            route: route,
            frozenOpeningPresentation: frozenOpeningPresentation,
            restoredRows: restoredRows
        )

        // The first delivered frame starts preparation; the second mounts the
        // live transcript behind the still-visible opening cover.
        route.presentedFrame(interval: nil)
        route.presentedFrame(interval: 1.0 / 120.0)
        XCTAssertEqual(route.renderPhase, .materializingConversation)
        let materializingFrame = visibleFrame(
            route: route,
            frozenOpeningPresentation: frozenOpeningPresentation,
            restoredRows: restoredRows
        )

        for _ in 0..<GaryxConversationRoutePresentationState.defaultMaterializationFrameCount {
            route.presentedFrame(interval: 1.0 / 120.0)
        }
        XCTAssertEqual(route.renderPhase, .live)
        let liveFrame = visibleFrame(
            route: route,
            frozenOpeningPresentation: frozenOpeningPresentation,
            restoredRows: restoredRows
        )

        let observed = [openingFrame, materializingFrame, liveFrame]
        XCTAssertEqual(
            observed.map(\.contentKinds),
            [
                [.loadingSkeleton],
                restoredRows.map { .realRow($0.id) } + [.loadingSkeleton],
                restoredRows.map { .realRow($0.id) },
            ],
            "the headless sequence must keep matching the production ZStack composition"
        )

        XCTAssertEqual(
            frozenOpeningPresentation,
            .localMessages,
            """
            EXPECTED FAILURE (#TASK-2644): a locally persisted existing thread \
            is frozen as loading because opening metadata only sees the empty \
            in-memory row store
            """
        )

        XCTAssertEqual(
            observed.filter(\.mixesSkeletonAndRealRows),
            [],
            """
            EXPECTED FAILURE (#TASK-2644): loading must be all-skeleton only \
            when there are zero local rows, or all-real when local rows exist; \
            materialization currently composites both kinds in one viewport
            """
        )
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

    /// Test-only observation of the existing production composition:
    ///
    /// - `mountsLiveTranscript` controls the first ZStack child;
    /// - `showsOpeningTranscriptCover` plus frozen `.loading` metadata controls
    ///   the transparent skeleton child above it.
    ///
    /// This small adapter is necessary because that final composition decision
    /// still lives in SwiftUI rather than in `GaryxMobileCore`.
    private func visibleFrame(
        route: GaryxConversationRoutePresentationState,
        frozenOpeningPresentation: GaryxConversationOpeningTranscriptPresentation,
        restoredRows: [GaryxMobileTurnRow]
    ) -> VisibleFrame {
        var kinds: [VisibleContentKind] = []
        if route.mountsLiveTranscript {
            kinds += restoredRows.map { .realRow($0.id) }
        }
        if route.showsOpeningTranscriptCover,
           frozenOpeningPresentation == .loading {
            kinds.append(.loadingSkeleton)
        }
        return VisibleFrame(phase: route.renderPhase, contentKinds: kinds)
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

    enum VisibleContentKind: Equatable, CustomStringConvertible {
        case loadingSkeleton
        case realRow(String)

        var description: String {
            switch self {
            case .loadingSkeleton:
                "skeleton"
            case .realRow(let id):
                "real(\(id))"
            }
        }
    }

    struct VisibleFrame: Equatable, CustomStringConvertible {
        let phase: GaryxConversationRouteRenderPhase
        let contentKinds: [VisibleContentKind]

        var mixesSkeletonAndRealRows: Bool {
            let hasSkeleton = contentKinds.contains(.loadingSkeleton)
            let hasRealRows = contentKinds.contains {
                if case .realRow = $0 { return true }
                return false
            }
            return hasSkeleton && hasRealRows
        }

        var description: String {
            "\(phase.rawValue):[\(contentKinds.map(\.description).joined(separator: ","))]"
        }
    }
}
