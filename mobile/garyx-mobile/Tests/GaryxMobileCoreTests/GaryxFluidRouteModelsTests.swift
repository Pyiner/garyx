import XCTest
@testable import GaryxMobileCore

final class GaryxFluidRouteModelsTests: XCTestCase {
    private let scope1 = GaryxGatewayScope(identity: "gateway-1", epoch: 1)
    private let scope2 = GaryxGatewayScope(identity: "gateway-2", epoch: 1)

    func testOccurrenceIdentityIsIndependentFromComposerKey() {
        var state = GaryxCanonicalRouteState()
        let first = entry("occurrence-a1", .conversation(threadID: "thread-a"))
        let panel = entry("panel", .panel("agents"))
        let second = entry("occurrence-a2", .conversation(threadID: "thread-a"))

        XCTAssertEqual(state.open(first), .appended(first.id))
        XCTAssertEqual(state.open(panel), .appended(panel.id))
        XCTAssertEqual(state.open(second), .appended(second.id))

        XCTAssertEqual(state.path.map(\.id), [first.id, panel.id, second.id])
        XCTAssertNotEqual(first.id, second.id)
        XCTAssertEqual(first.destination.composerKey, second.destination.composerKey)
        XCTAssertEqual(state.predecessorNode, .entry(panel))
    }

    func testDraftHasSingleOccurrenceAndFocusesItWithoutDuplicating() {
        var state = GaryxCanonicalRouteState()
        let draft = entry("draft-occurrence", .conversationDraft(draftID: "draft-1"))
        _ = state.open(draft)
        _ = state.open(entry("panel", .panel("settings")))
        let revisionBeforeFocus = state.stackRevision

        XCTAssertEqual(
            state.open(entry("ignored-new-id", .conversationDraft(draftID: "draft-1"))),
            .focusedExistingDraft(draft.id)
        )
        XCTAssertEqual(state.path, [draft])
        XCTAssertEqual(state.stackRevision, revisionBeforeFocus + 1)
    }

    func testHomeAndFirstLevelPredecessorAreExplicitPresentationNodes() {
        var state = GaryxCanonicalRouteState()
        XCTAssertEqual(state.topNode, .home)
        XCTAssertEqual(state.predecessorNode, .home)

        let route = entry("one", .panel("agents"))
        _ = state.open(route)
        XCTAssertEqual(state.topNode, .entry(route))
        XCTAssertEqual(state.predecessorNode, .home)
        _ = state.pop()
        XCTAssertEqual(state.topNode, .home)
    }

    func testPayloadRevisionAndStackRevisionHaveSeparateAuthorities() {
        var state = GaryxCanonicalRouteState()
        let draft = entry("draft-occurrence", .conversationDraft(draftID: "draft-1"))
        _ = state.open(draft)
        let topologyRevision = state.stackRevision

        let result = state.promoteDraft(
            promotion(stage: .serverAcknowledged),
            currentScope: scope1,
            outboxAdmission: .succeeded
        )

        XCTAssertEqual(result.navigation, .updatedInPlace)
        XCTAssertEqual(state.stackRevision, topologyRevision)
        XCTAssertEqual(state.path[0].payloadRevision, 1)
        XCTAssertEqual(state.path[0].destination, .conversation(threadID: "thread-1"))

        _ = state.open(entry("another", .panel("skills")))
        XCTAssertEqual(state.stackRevision, topologyRevision + 1)
        XCTAssertEqual(state.path[0].payloadRevision, 1)
    }

    func testLatePromotionMigratesDomainWithoutReinsertingPath() {
        var state = GaryxCanonicalRouteState()
        _ = state.open(entry("draft-occurrence", .conversationDraft(draftID: "draft-1")))
        _ = state.pop()
        let topologyRevision = state.stackRevision

        let result = state.promoteDraft(
            promotion(stage: .serverAcknowledged),
            currentScope: scope1,
            outboxAdmission: .succeeded
        )

        XCTAssertEqual(result.navigation, .domainOnlyLate)
        XCTAssertTrue(result.migratedDomainInOriginScope)
        XCTAssertTrue(state.path.isEmpty)
        XCTAssertEqual(state.stackRevision, topologyRevision)
    }

    func testPromotionFromSuspendedOriginCannotMutateCurrentScopePathOrLease() {
        var state = GaryxCanonicalRouteState()
        let original = entry("draft-occurrence", .conversationDraft(draftID: "draft-1"))
        _ = state.open(original)
        let revision = state.stackRevision

        let result = state.promoteDraft(
            promotion(stage: .serverAcknowledged),
            currentScope: scope2,
            outboxAdmission: .succeeded
        )

        XCTAssertEqual(result.navigation, .originScopePartitionOnly)
        XCTAssertTrue(result.migratedDomainInOriginScope)
        XCTAssertTrue(result.preservedPresentationLease)
        XCTAssertEqual(state.path, [original])
        XCTAssertEqual(state.stackRevision, revision)
    }

    func testPromotionSendStageTransferTableNeverDispatchesAgain() {
        let cases: [(GaryxDraftPromotionSendStage, GaryxDraftPromotionSendDisposition, Int)] = [
            (.threadCreatedButNotDispatched, .failedRetryableOutbox, 1),
            (.dispatchInFlight, .reconcileAmbiguous, 0),
            (.serverAcknowledged, .acknowledged, 0),
        ]

        for (stage, expectedSend, expectedOutbox) in cases {
            var state = GaryxCanonicalRouteState()
            _ = state.open(entry("draft-occurrence", .conversationDraft(draftID: "draft-1")))
            let result = state.promoteDraft(
                promotion(stage: stage),
                currentScope: scope1,
                outboxAdmission: .succeeded
            )
            XCTAssertEqual(result.send, expectedSend, "stage=\(stage)")
            XCTAssertEqual(result.outboxInsertCount, expectedOutbox, "stage=\(stage)")
            XCTAssertEqual(result.dispatchCountDelta, 0, "promotion never redispatches")
            XCTAssertTrue(result.keptOptimisticThread)
        }
    }

    func testOutboxPersistenceFailureAtomicallyKeepsDraftAuthoritative() {
        var state = GaryxCanonicalRouteState()
        let draft = entry("draft-occurrence", .conversationDraft(draftID: "draft-1"))
        _ = state.open(draft)

        let result = state.promoteDraft(
            promotion(stage: .threadCreatedButNotDispatched),
            currentScope: scope1,
            outboxAdmission: .failed(code: "fsync_failed")
        )

        XCTAssertEqual(result.navigation, .draftRestored)
        XCTAssertEqual(result.send, .typedFailure(code: "fsync_failed"))
        XCTAssertFalse(result.migratedDomainInOriginScope)
        XCTAssertFalse(result.keptOptimisticThread)
        XCTAssertEqual(result.outboxInsertCount, 0)
        XCTAssertEqual(state.path, [draft])
    }

    private func entry(
        _ id: String,
        _ destination: GaryxRouteDestination
    ) -> GaryxRouteEntry {
        GaryxRouteEntry(id: GaryxRouteInstanceID(rawValue: id), destination: destination)
    }

    private func promotion(stage: GaryxDraftPromotionSendStage) -> GaryxDraftPromotionRequest {
        GaryxDraftPromotionRequest(
            instanceID: GaryxRouteInstanceID(rawValue: "draft-occurrence"),
            draftID: "draft-1",
            threadID: "thread-1",
            originScope: scope1,
            clientIntentID: "intent-1",
            sendStage: stage
        )
    }
}
