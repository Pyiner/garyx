import XCTest
@testable import GaryxMobileCore

final class GaryxCapsulePreviewLoadingTests: XCTestCase {
    func testProjectionRevisionChangeAndPresentToMissingChangeLoadKey() {
        let selection = GaryxCapsulePreviewSelection(capsule: capsule(revision: 1))
        let rev1 = GaryxCapsulePreviewProjection.loadKey(
            selection: selection,
            catalog: [capsule(revision: 1)],
            retryGeneration: 0
        )
        let rev2 = GaryxCapsulePreviewProjection.loadKey(
            selection: selection,
            catalog: [capsule(revision: 2)],
            retryGeneration: 0
        )
        let missing = GaryxCapsulePreviewProjection.loadKey(
            selection: selection,
            catalog: [],
            retryGeneration: 0
        )

        XCTAssertEqual(rev1.projectedRevision, 1)
        XCTAssertEqual(rev2.projectedRevision, 2)
        XCTAssertNil(missing.projectedRevision)
        XCTAssertNotEqual(rev1, rev2)
        XCTAssertNotEqual(rev2, missing)
        XCTAssertEqual(
            GaryxCapsulePreviewProjection.displaySummary(selection: selection, catalog: []),
            selection.fallback
        )
    }

    func testRetryGenerationChangesKeyAtSameRevision() {
        let selection = GaryxCapsulePreviewSelection(capsule: capsule(revision: 4))
        let first = GaryxCapsulePreviewProjection.loadKey(
            selection: selection,
            catalog: [capsule(revision: 4)],
            retryGeneration: 0
        )
        let retried = GaryxCapsulePreviewProjection.loadKey(
            selection: selection,
            catalog: [capsule(revision: 4)],
            retryGeneration: 1
        )
        XCTAssertNotEqual(first, retried)
        XCTAssertEqual(retried.retryGeneration, 1)
    }

    func testLoadStatusRetryDecisionIsIndependentFromRenderedContent() {
        let key = GaryxCapsulePreviewLoadKey(id: "capsule-1", projectedRevision: 2, retryGeneration: 0)
        let status = GaryxCapsulePreviewLoadStatus(
            requestedKey: key,
            attempt: 1,
            phase: .failed,
            failure: .init(kind: .retryable, message: "temporary")
        )
        let rendered = GaryxCapsulePreviewRenderedContent(html: "<p>rev1</p>", revision: 1)

        XCTAssertTrue(status.isRetryableFailure(for: key))
        XCTAssertEqual(rendered.revision, 1)
    }

    func testRetryReducerUsesTwoFiveTenAndStopsAfterFourAttempts() {
        var state = GaryxCapsulePreviewRetryState()
        let failure = GaryxCapsulePreviewFailure(kind: .retryable, message: "offline")
        _ = GaryxCapsulePreviewRetryReducer.reduce(state: &state, event: .beginCycle)

        for (attempt, expectedDelay) in [2.0, 5.0, 10.0].enumerated() {
            _ = GaryxCapsulePreviewRetryReducer.reduce(state: &state, event: .attemptStarted)
            XCTAssertEqual(state.networkAttempt, attempt + 1)
            XCTAssertEqual(
                GaryxCapsulePreviewRetryReducer.reduce(state: &state, event: .failed(failure)),
                .retry(after: expectedDelay)
            )
            _ = GaryxCapsulePreviewRetryReducer.reduce(state: &state, event: .retryDelayElapsed)
        }

        _ = GaryxCapsulePreviewRetryReducer.reduce(state: &state, event: .attemptStarted)
        XCTAssertEqual(state.networkAttempt, 4)
        XCTAssertEqual(
            GaryxCapsulePreviewRetryReducer.reduce(state: &state, event: .failed(failure)),
            .none
        )
        XCTAssertEqual(state.phase, .exhausted)
    }

    func testRetryAfterWinsOverBackoffSlot() {
        var state = GaryxCapsulePreviewRetryState()
        _ = GaryxCapsulePreviewRetryReducer.reduce(state: &state, event: .beginCycle)
        _ = GaryxCapsulePreviewRetryReducer.reduce(state: &state, event: .attemptStarted)
        let failure = GaryxCapsulePreviewFailure(
            kind: .retryable,
            message: "rate limited",
            retryAfter: 9
        )
        XCTAssertEqual(
            GaryxCapsulePreviewRetryReducer.reduce(state: &state, event: .failed(failure)),
            .retry(after: 9)
        )
    }

    func testTerminalAndDeletedFailuresNeverRetry() {
        for kind in [GaryxCapsulePreviewFailureKind.terminal, .deleted] {
            var state = GaryxCapsulePreviewRetryState()
            _ = GaryxCapsulePreviewRetryReducer.reduce(state: &state, event: .beginCycle)
            _ = GaryxCapsulePreviewRetryReducer.reduce(state: &state, event: .attemptStarted)
            XCTAssertEqual(
                GaryxCapsulePreviewRetryReducer.reduce(
                    state: &state,
                    event: .failed(.init(kind: kind, message: "stop"))
                ),
                .none
            )
            XCTAssertEqual(state.phase, kind == .deleted ? .deleted : .terminalFailure)
        }
    }

    func testSceneEventsCancelAndActiveStartsFreshCycle() {
        var state = GaryxCapsulePreviewRetryState()
        _ = GaryxCapsulePreviewRetryReducer.reduce(state: &state, event: .beginCycle)
        _ = GaryxCapsulePreviewRetryReducer.reduce(state: &state, event: .attemptStarted)
        XCTAssertEqual(
            GaryxCapsulePreviewRetryReducer.reduce(state: &state, event: .sceneInactive),
            .cancel
        )
        XCTAssertEqual(state.phase, .cancelled)
        let priorCycle = state.cycleGeneration
        XCTAssertEqual(
            GaryxCapsulePreviewRetryReducer.reduce(state: &state, event: .sceneActive),
            .none
        )
        XCTAssertEqual(state.cycleGeneration, priorCycle + 1)
        XCTAssertEqual(state.networkAttempt, 0)
        XCTAssertEqual(state.phase, .running)
    }

    func testCanonicalFailureClassification() {
        XCTAssertEqual(
            GaryxCapsulePreviewFailure.classify(
                GaryxGatewayError.httpStatus(429, "busy", retryAfter: 8)
            ),
            .init(kind: .retryable, message: "busy", retryAfter: 8)
        )
        XCTAssertEqual(
            GaryxCapsulePreviewFailure.classify(GaryxGatewayError.httpStatus(404, "gone"))?.kind,
            .deleted
        )
        XCTAssertEqual(
            GaryxCapsulePreviewFailure.classify(GaryxGatewayError.httpStatus(403, "forbidden"))?.kind,
            .terminal
        )
        XCTAssertNil(GaryxCapsulePreviewFailure.classify(CancellationError()))
    }

    private func capsule(revision: Int) -> GaryxCapsuleSummary {
        GaryxCapsuleSummary(id: "capsule-1", title: "Synthetic Capsule", revision: revision)
    }
}
