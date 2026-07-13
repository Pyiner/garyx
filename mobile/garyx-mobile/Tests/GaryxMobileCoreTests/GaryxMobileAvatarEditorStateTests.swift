import XCTest
@testable import GaryxMobileCore

final class GaryxMobileAvatarEditorStateTests: XCTestCase {
    func testTypedFailureClassificationPreservesStatusAndCancellation() {
        XCTAssertEqual(
            GaryxAvatarGenerationOutcome.from(
                error: GaryxGatewayError.httpStatus(504, "timeout")
            ),
            .failure(GaryxAvatarGenerationFailure(category: .timeout))
        )
        XCTAssertEqual(
            GaryxAvatarGenerationOutcome.from(
                error: GaryxGatewayError.httpStatus(502, "provider")
            ),
            .failure(GaryxAvatarGenerationFailure(category: .provider))
        )
        XCTAssertEqual(
            GaryxAvatarGenerationOutcome.from(error: CancellationError()),
            .cancelled
        )
    }

    private let firstRequest = UUID(uuidString: "00000000-0000-0000-0000-000000000101")!
    private let secondRequest = UUID(uuidString: "00000000-0000-0000-0000-000000000102")!

    func testChoosingGeneratingCandidateAndUseKeepCurrentUntilConfirmation() throws {
        var state = GaryxMobileAvatarEditorState(currentAvatarDataUrl: "current")
        XCTAssertEqual(state.phase, .choosing)
        XCTAssertEqual(state.primaryAction, .generate)
        XCTAssertEqual(state.leadingAction, .cancel)

        XCTAssertEqual(state.beginGeneration(requestId: firstRequest), firstRequest)
        XCTAssertEqual(state.phase, .generating)
        XCTAssertEqual(state.primaryAction, .disabled)
        XCTAssertEqual(state.leadingAction, .cancelGeneration)

        XCTAssertTrue(state.resolve(.success(dataUrl: "new"), requestId: firstRequest))
        XCTAssertEqual(state.phase, .candidate)
        XCTAssertEqual(state.currentAvatarDataUrl, "current")
        XCTAssertEqual(state.candidateAvatarDataUrl, "new")
        XCTAssertEqual(state.primaryAction, .use)

        XCTAssertEqual(try XCTUnwrap(state.acceptCandidate()), "new")
        XCTAssertEqual(state.currentAvatarDataUrl, "new")
        XCTAssertEqual(state.phase, .choosing)
    }

    func testFailureRetryPreservesPriorCandidate() {
        var state = GaryxMobileAvatarEditorState(
            currentAvatarDataUrl: "current",
            candidateAvatarDataUrl: "prior",
            phase: .candidate
        )
        _ = state.beginGeneration(requestId: firstRequest)
        let failure = GaryxAvatarGenerationFailure(category: .provider)

        XCTAssertTrue(state.resolve(.failure(failure), requestId: firstRequest))
        XCTAssertEqual(state.phase, .failed(failure))
        XCTAssertEqual(state.candidateAvatarDataUrl, "prior")
        XCTAssertEqual(state.primaryAction, .retry)

        XCTAssertEqual(state.beginGeneration(requestId: secondRequest), secondRequest)
        XCTAssertEqual(state.phase, .generating)
        XCTAssertEqual(state.candidateAvatarDataUrl, "prior")
    }

    func testFailedChangeStyleReturnsToChoosingAndKeepsStyleAndCandidate() {
        let failure = GaryxAvatarGenerationFailure(category: .timeout)
        var state = GaryxMobileAvatarEditorState(
            currentAvatarDataUrl: "current",
            candidateAvatarDataUrl: "prior",
            phase: .failed(failure),
            selectedStyleId: "custom",
            customStyle: "paper cut"
        )

        state.changeStyle()

        XCTAssertEqual(state.phase, .choosing)
        XCTAssertEqual(state.selectedStyleId, "custom")
        XCTAssertEqual(state.customStyle, "paper cut")
        XCTAssertEqual(state.candidateAvatarDataUrl, "prior")
    }

    func testRetryRejectsLateSuccessFailureAndFinallyFromOldRequest() {
        var state = GaryxMobileAvatarEditorState(currentAvatarDataUrl: "current")
        _ = state.beginGeneration(requestId: firstRequest)
        XCTAssertTrue(state.cancelGeneration(requestId: firstRequest))
        _ = state.beginGeneration(requestId: secondRequest)

        XCTAssertFalse(state.resolve(.success(dataUrl: "late"), requestId: firstRequest))
        XCTAssertFalse(
            state.resolve(
                .failure(GaryxAvatarGenerationFailure(category: .unknown)),
                requestId: firstRequest
            )
        )
        XCTAssertFalse(state.cancelGeneration(requestId: firstRequest))
        XCTAssertEqual(state.requestId, secondRequest)
        XCTAssertEqual(state.phase, .generating)

        XCTAssertTrue(state.resolve(.success(dataUrl: "latest"), requestId: secondRequest))
        XCTAssertEqual(state.candidateAvatarDataUrl, "latest")
    }

    func testCancelClearsOwnershipAndLateResultCannotApply() {
        var state = GaryxMobileAvatarEditorState(currentAvatarDataUrl: "current")
        _ = state.beginGeneration(requestId: firstRequest)

        XCTAssertTrue(state.cancelGeneration())
        XCTAssertEqual(state.phase, .choosing)
        XCTAssertNil(state.requestId)
        XCTAssertFalse(state.resolve(.success(dataUrl: "late"), requestId: firstRequest))
        XCTAssertEqual(state.currentAvatarDataUrl, "current")
        XCTAssertNil(state.candidateAvatarDataUrl)
    }

    func testCancelledAndSupersededOutcomesNeverBecomeFailures() {
        for outcome in [GaryxAvatarGenerationOutcome.cancelled, .superseded] {
            var state = GaryxMobileAvatarEditorState(currentAvatarDataUrl: "current")
            _ = state.beginGeneration(requestId: firstRequest)
            XCTAssertTrue(state.resolve(outcome, requestId: firstRequest))
            XCTAssertEqual(state.phase, .choosing)
        }
    }

    func testToolbarPrimaryActionExhaustivelyMatchesPhaseContract() {
        let failure = GaryxAvatarGenerationFailure(category: .unreachable)
        XCTAssertEqual(GaryxMobileAvatarEditorState(phase: .choosing).primaryAction, .generate)
        XCTAssertEqual(GaryxMobileAvatarEditorState(phase: .generating).primaryAction, .disabled)
        XCTAssertEqual(GaryxMobileAvatarEditorState(phase: .candidate).primaryAction, .use)
        XCTAssertEqual(GaryxMobileAvatarEditorState(phase: .failed(failure)).primaryAction, .retry)
    }

    func testCustomStyleMustBeNonEmptyAndBuiltInStyleResolvesPrompt() {
        var state = GaryxMobileAvatarEditorState(selectedStyleId: "custom")
        XCTAssertFalse(state.canGenerate)
        state.customStyle = "  enamel badge  "
        XCTAssertEqual(state.activeStylePrompt, "enamel badge")
        XCTAssertTrue(state.canGenerate)

        state.selectedStyleId = "paper_cut"
        XCTAssertEqual(
            state.activeStylePrompt,
            GaryxAvatarStyleOption.builtIn.first(where: { $0.id == "paper_cut" })?.prompt
        )
    }
}
