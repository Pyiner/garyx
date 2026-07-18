import Foundation
import XCTest
@testable import GaryxMobileCore

final class GaryxLifecycleMutationStateTests: XCTestCase {
    private let request = GaryxLifecycleMutationRequest(
        kind: .archive,
        threadId: "thread::lifecycle",
        endpointKeys: ["telegram::main::1000000001"],
        operationId: UUID(uuidString: "10000000-0000-4000-8000-000000000001")!,
        expectedStoreIncarnation: "20000000-0000-4000-8000-000000000002",
        gatewayScope: "gateway-scope",
        gatewayRequestToken: GaryxGatewayRequestToken(
            scope: GaryxGatewayScope(identity: "gateway-scope", epoch: 3),
            activationSequence: 1
        )
    )

    func testRealTimingPolicyKeepsJoinInsideTransportAndDefinesFiveResends() {
        XCTAssertEqual(GaryxLifecycleMutationPolicy.joinWindowSeconds, 6)
        XCTAssertEqual(GaryxLifecycleMutationPolicy.transportTimeoutSeconds, 8)
        XCTAssertLessThan(
            GaryxLifecycleMutationPolicy.joinWindowSeconds,
            GaryxLifecycleMutationPolicy.transportTimeoutSeconds
        )
        XCTAssertEqual(
            GaryxLifecycleMutationPolicy.retryDelaysNanoseconds,
            [
                1_000_000_000,
                2_000_000_000,
                4_000_000_000,
                8_000_000_000,
                8_000_000_000,
            ]
        )
    }

    func testInProgressAndAmbiguousReuseOneIdentityBeforeApplied() {
        var state = GaryxLifecycleMutationState(request: request)
        let first = state.nextAttempt()!
        XCTAssertEqual(first.attemptNumber, 1)
        XCTAssertEqual(
            state.settle(tagged("operation_in_progress")),
            .retry(delayNanoseconds: 1_000_000_000)
        )
        let second = state.nextAttempt()!
        XCTAssertEqual(
            state.settle(
                GaryxGatewayMutationResult<GaryxArchiveThreadResult>.ambiguous(
                    GaryxGatewayAmbiguousResponse(
                        message: "response lost",
                        status: nil,
                        body: nil
                    )
                )
            ),
            .retry(delayNanoseconds: 2_000_000_000)
        )
        let third = state.nextAttempt()!
        let applied = archiveResult()
        XCTAssertEqual(state.settle(.ok(applied)), .applied(applied))
        XCTAssertEqual(state.attemptCount, 3)
        XCTAssertEqual(first.request.operationId, second.request.operationId)
        XCTAssertEqual(second.request.operationId, third.request.operationId)
        XCTAssertEqual(
            first.request.expectedStoreIncarnation,
            third.request.expectedStoreIncarnation
        )
    }

    func testFiveBackoffResendsExhaustAfterSixSingleAttempts() {
        var state = GaryxLifecycleMutationState(request: request)
        for expectedDelay in GaryxLifecycleMutationPolicy.retryDelaysNanoseconds {
            XCTAssertNotNil(state.nextAttempt())
            XCTAssertEqual(
                state.settle(ambiguous("lost")),
                .retry(delayNanoseconds: expectedDelay)
            )
        }
        XCTAssertEqual(state.nextAttempt()?.attemptNumber, 6)
        XCTAssertEqual(state.settle(ambiguous("lost-final")), .exhausted(message: "lost-final"))
        XCTAssertNil(state.nextAttempt())
    }

    func testDeterministicThreeWayCompletionAndConflictTerminal() {
        var appliedState = GaryxLifecycleMutationState(request: request)
        _ = appliedState.nextAttempt()
        let value = archiveResult()
        XCTAssertEqual(appliedState.settle(.ok(value)), .applied(value))

        for code in ["rejected_conflict", "rejected_not_found", "wrong_incarnation"] {
            var rejectedState = GaryxLifecycleMutationState(request: request)
            _ = rejectedState.nextAttempt()
            XCTAssertEqual(
                rejectedState.settle(tagged(code)),
                .rejected(code: code, message: code)
            )
            XCTAssertNil(rejectedState.nextAttempt())
        }

        var conflictState = GaryxLifecycleMutationState(request: request)
        _ = conflictState.nextAttempt()
        XCTAssertEqual(
            conflictState.settle(tagged("operation_id_conflict")),
            .operationIdConflict(message: "operation_id_conflict")
        )
        XCTAssertNil(conflictState.nextAttempt())
    }

    func testUnavailableAndNotSentUseTheSameBoundedRetryLane() {
        var unavailable = GaryxLifecycleMutationState(request: request)
        _ = unavailable.nextAttempt()
        XCTAssertEqual(
            unavailable.settle(tagged("unavailable")),
            .retry(delayNanoseconds: 1_000_000_000)
        )

        var notSent = GaryxLifecycleMutationState(request: request)
        _ = notSent.nextAttempt()
        XCTAssertEqual(
            notSent.settle(
                GaryxGatewayMutationResult<GaryxArchiveThreadResult>.notSent("offline")
            ),
            .retry(delayNanoseconds: 1_000_000_000)
        )
    }

    private func ambiguous(
        _ message: String
    ) -> GaryxGatewayMutationResult<GaryxArchiveThreadResult> {
        .ambiguous(
            GaryxGatewayAmbiguousResponse(message: message, status: nil, body: nil)
        )
    }

    private func tagged(
        _ code: String
    ) -> GaryxGatewayMutationResult<GaryxArchiveThreadResult> {
        .definitiveEndpointResponse(
            GaryxGatewayDefinitiveEndpointResponse(
                status: code == "unavailable" ? 503 : 409,
                error: GaryxGatewayTaggedAPIError(
                    kind: "garyx_api_error",
                    operation: "thread_archive",
                    code: code,
                    message: code
                ),
                decoded: nil,
                body: Data()
            )
        )
    }

    private func archiveResult() -> GaryxArchiveThreadResult {
        try! JSONDecoder().decode(
            GaryxArchiveThreadResult.self,
            from: Data(
                """
                {
                  "archived": true,
                  "deleted": true,
                  "changed": true,
                  "operation_id": "10000000-0000-4000-8000-000000000001",
                  "outcome": "applied_changed",
                  "thread_id": "thread::lifecycle",
                  "detached_endpoint_keys": []
                }
                """.utf8
            )
        )
    }
}
