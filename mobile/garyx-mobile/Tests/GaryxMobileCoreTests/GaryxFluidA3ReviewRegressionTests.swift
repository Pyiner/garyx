import XCTest
@testable import GaryxMobileCore

final class GaryxFluidA3ReviewRegressionTests: XCTestCase {
    private let scope = GaryxGatewayScope(identity: "gateway", epoch: 1)

    func testIdentityInvalidCompletionPreservesEveryTerminalOperationState() {
        for terminalState in [
            GaryxOperationCapabilityState.completed,
            .failedTerminal,
            .cancelled,
            .superseded,
        ] {
            var operation = makeOperation(
                entryID: entryID("entry-\(terminalState.rawValue)"),
                operationID: "operation-\(terminalState.rawValue)",
                state: terminalState,
                stagedAssetID: GaryxStagedAssetID(rawValue: "asset-\(terminalState.rawValue)"),
                reservedBytes: 8
            )
            let key = operation.context.key
            let lifecycle = activeLifecycle(for: key.entryID)
            operation.invalidateIdentity()

            XCTAssertEqual(
                operation.complete(
                    expectedKey: key,
                    lifecycle: lifecycle,
                    scopes: activeScopes()
                ),
                .archivedIdentityInvalid,
                "state=\(terminalState)"
            )
            XCTAssertEqual(operation.state, terminalState, "terminal state must be immutable")
            XCTAssertNil(operation.stagedAssetID)
            XCTAssertEqual(operation.reservedBytes, 0)
        }
    }

    func testOldCommittedReservationResultBeforeProducerTerminalStillReachesDraft() {
        var state = makeInputState(text: "T")
        let lifecycle = activeLifecycle(for: state)
        let scopes = activeScopes()
        let reservation = GaryxSendReservationID(rawValue: 1)

        XCTAssertEqual(
            state.beginSend(
                reservationID: reservation,
                followupGeneration: 11,
                lifecycle: lifecycle,
                scopes: scopes
            ),
            .sealed(envelope: "T", followupGeneration: 11)
        )
        XCTAssertTrue(state.commitReservation(lifecycle: lifecycle, scopes: scopes))
        XCTAssertTrue(state.returnReservationToIdle(lifecycle: lifecycle, scopes: scopes))
        XCTAssertEqual(state.producerPhase, .live)

        let lateDictation = GaryxComposerInputEventIdentity(
            composerKey: state.session.composerKey,
            sessionID: state.session.sessionID,
            inputSessionEpoch: state.session.epoch,
            payloadGeneration: 11,
            reservationID: reservation,
            inputSequence: 1
        )
        XCTAssertEqual(
            state.applyText(
                "late dictation",
                identity: lateDictation,
                lifecycle: lifecycle,
                scopes: scopes
            ),
            .applied(target: .committedNextGeneration, generation: 11)
        )
        XCTAssertEqual(state.currentText, "late dictation")
    }

    func testOldRevokedReservationResultBeforeProducerTerminalPreservesEnvelope() {
        var state = makeInputState(text: "T")
        let lifecycle = activeLifecycle(for: state)
        let scopes = activeScopes()
        let reservation = GaryxSendReservationID(rawValue: 1)

        XCTAssertEqual(
            state.beginSend(
                reservationID: reservation,
                followupGeneration: 11,
                lifecycle: lifecycle,
                scopes: scopes
            ),
            .sealed(envelope: "T", followupGeneration: 11)
        )
        XCTAssertEqual(
            state.applyText(
                "U",
                identity: GaryxComposerInputEventIdentity(
                    composerKey: state.session.composerKey,
                    sessionID: state.session.sessionID,
                    inputSessionEpoch: state.session.epoch,
                    payloadGeneration: 11,
                    reservationID: reservation,
                    inputSequence: 1
                ),
                lifecycle: lifecycle,
                scopes: scopes
            ),
            .applied(target: .provisionalNextGeneration, generation: 11)
        )
        XCTAssertTrue(
            state.revokeReservation(
                mergeGeneration: 12,
                lifecycle: lifecycle,
                scopes: scopes
            )
        )
        XCTAssertEqual(state.currentText, "TU")
        XCTAssertTrue(state.returnReservationToIdle(lifecycle: lifecycle, scopes: scopes))
        XCTAssertEqual(state.producerPhase, .live)

        let lateDictation = GaryxComposerInputEventIdentity(
            composerKey: state.session.composerKey,
            sessionID: state.session.sessionID,
            inputSessionEpoch: state.session.epoch,
            payloadGeneration: 11,
            reservationID: reservation,
            inputSequence: 2
        )
        XCTAssertEqual(
            state.applyText(
                "Ufinal",
                identity: lateDictation,
                lifecycle: lifecycle,
                scopes: scopes
            ),
            .applied(target: .revokedMergeGeneration, generation: 12)
        )
        XCTAssertEqual(state.currentText, "TUfinal")

        XCTAssertEqual(
            state.applyText(
                "Usettled",
                identity: GaryxComposerInputEventIdentity(
                    composerKey: state.session.composerKey,
                    sessionID: state.session.sessionID,
                    inputSessionEpoch: state.session.epoch,
                    payloadGeneration: 11,
                    reservationID: reservation,
                    inputSequence: 3
                ),
                lifecycle: lifecycle,
                scopes: scopes
            ),
            .applied(target: .revokedMergeGeneration, generation: 12)
        )
        XCTAssertEqual(
            state.currentText,
            "TUsettled",
            "each producer snapshot replaces the mutable suffix without duplicating the envelope"
        )
    }

    func testSendPromotionFollowupRoutesThroughRetiringAliasIntoStableEntry() {
        var input = makeInputState(text: "S1")
        let lifecycle = activeLifecycle(for: input)
        let scopes = activeScopes()
        let reservation = GaryxSendReservationID(rawValue: 1)
        _ = input.beginSend(
            reservationID: reservation,
            followupGeneration: 11,
            lifecycle: lifecycle,
            scopes: scopes
        )
        XCTAssertTrue(input.commitReservation(lifecycle: lifecycle, scopes: scopes))
        XCTAssertTrue(input.returnReservationToIdle(lifecycle: lifecycle, scopes: scopes))

        let entry = makeEntry(
            id: "input-entry",
            destination: .draft("draft"),
            generation: 11
        )
        var store = GaryxComposerPayloadStore()
        XCTAssertTrue(store.insert(entry))
        XCTAssertTrue(store.promote(entryID: entry.id, scope: scope, to: .thread("thread")))
        var aliases = GaryxComposerAliasTable()
        XCTAssertEqual(
            aliases.establishPromotion(
                scope: scope,
                source: .draft("draft"),
                target: .thread("thread"),
                activeOrClosingSessions: 1
            ),
            .established
        )

        let followup = GaryxComposerInputEventIdentity(
            composerKey: .thread("thread"),
            sessionID: input.session.sessionID,
            inputSessionEpoch: input.session.epoch,
            payloadGeneration: 11,
            reservationID: nil,
            inputSequence: 1
        )
        XCTAssertEqual(
            input.applyText(
                "S2 follow-up",
                identity: followup,
                aliases: aliases,
                lifecycle: lifecycle,
                scopes: scopes
            ),
            .applied(target: .currentGeneration, generation: 11)
        )
        XCTAssertEqual(input.currentText, "S2 follow-up")
        XCTAssertEqual(store.entry(entry.id, scope: scope)?.destination, .thread("thread"))
        XCTAssertEqual(aliases.aliasCount, 1, "retiring alias stays live until the old session drains")
        XCTAssertTrue(aliases.markDrained(source: .draft("draft"), scope: scope))
        XCTAssertEqual(aliases.aliasCount, 0)
    }

    func testFailedRetryableRevokedUsesCancelledChildCleanupSemantics() {
        XCTAssertEqual(
            GaryxOperationRecoveryPlanner.decide(
                state: .failedRetryable,
                uploadAttempted: true,
                scope: .revoked
            ),
            .cleanOperationChild
        )
    }

    func testPayloadStoreRejectsUncoordinatedPromotionCollision() {
        let source = makeEntry(id: "source", destination: .draft("draft"), text: "source")
        let target = makeEntry(id: "target", destination: .thread("thread"), text: "target")
        var store = GaryxComposerPayloadStore()
        XCTAssertTrue(store.insert(source))
        XCTAssertTrue(store.insert(target))

        XCTAssertFalse(
            store.promote(entryID: source.id, scope: scope, to: .thread("thread")),
            "colliding promotion must go through durable PayloadConflictSet admission"
        )
        XCTAssertEqual(store.entry(source.id, scope: scope)?.destination, .draft("draft"))
    }

    func testPromotionCollisionDetectorAtomicallyAdmitsEveryCandidate() {
        let source = makeEntry(id: "source", destination: .draft("draft"), text: "source")
        let firstTarget = makeEntry(id: "target-a", destination: .thread("thread"), text: "A")
        let secondTarget = makeEntry(id: "target-b", destination: .thread("thread"), text: "B")
        var store = GaryxComposerPayloadStore()
        XCTAssertTrue(store.insert(source))
        XCTAssertTrue(store.insert(firstTarget))
        XCTAssertTrue(store.insert(secondTarget))
        var conflicts: [GaryxPayloadConflictSetID: GaryxPayloadConflictSet] = [:]
        let conflictID = GaryxPayloadConflictSetID(rawValue: "promotion-thread")

        let unavailable = GaryxPayloadPromotionReducer.promote(
            entryID: source.id,
            scope: scope,
            to: .thread("thread"),
            conflictSetID: conflictID,
            membershipDurabilityAvailable: false,
            store: &store,
            conflictSets: &conflicts
        )
        XCTAssertEqual(unavailable, .rejectedConflictDurability)
        XCTAssertEqual(store.entry(source.id, scope: scope)?.destination, .draft("draft"))
        XCTAssertTrue(conflicts.isEmpty)

        let admitted = GaryxPayloadPromotionReducer.promote(
            entryID: source.id,
            scope: scope,
            to: .thread("thread"),
            conflictSetID: conflictID,
            membershipDurabilityAvailable: true,
            store: &store,
            conflictSets: &conflicts
        )
        let expectedCandidates = [source.id, firstTarget.id, secondTarget.id]
            .sorted { $0.rawValue < $1.rawValue }
        XCTAssertEqual(
            admitted,
            .conflictAdmitted(conflictSetID: conflictID, candidates: expectedCandidates)
        )
        XCTAssertEqual(store.entry(source.id, scope: scope)?.destination, .thread("thread"))
        XCTAssertEqual(conflicts[conflictID]?.candidates.map(\.entryID), expectedCandidates)
        XCTAssertTrue(conflicts[conflictID]?.pendingDecision == true)
    }

    func testRevokedOriginPromotionCannotMigrateIntoDeadPartition() {
        var scopes = activeScopes()
        XCTAssertTrue(scopes.revoke(scope))
        let draft = GaryxRouteEntry(
            id: GaryxRouteInstanceID(rawValue: "draft-occurrence"),
            destination: .conversationDraft(draftID: "draft")
        )
        var route = GaryxCanonicalRouteState(path: [draft], stackRevision: 1)
        let result = route.promoteDraft(
            GaryxDraftPromotionRequest(
                instanceID: draft.id,
                draftID: "draft",
                threadID: "thread",
                originScope: scope,
                clientIntentID: "intent",
                sendStage: .threadCreatedButNotDispatched
            ),
            scopes: scopes,
            outboxAdmission: .succeeded
        )

        XCTAssertEqual(result.navigation, .originScopeRevoked)
        XCTAssertEqual(result.send, .rejectedRevokedScope)
        XCTAssertFalse(result.migratedDomainInOriginScope)
        XCTAssertFalse(result.keptOptimisticThread)
        XCTAssertEqual(result.outboxInsertCount, 0)
        XCTAssertEqual(route.path, [draft])
    }

    func testAliasDrainCannotLeaveAZeroOwnerAliasBehind() {
        var aliases = GaryxComposerAliasTable()
        aliases.establishPromotion(
            scope: scope,
            source: .draft("draft"),
            target: .thread("thread"),
            activeOrClosingSessions: 1
        )
        XCTAssertEqual(aliases.aliasCount, 1)
        XCTAssertEqual(aliases.activeRetiringSourceCount, 1)

        aliases.markDrained(source: .draft("draft"), scope: scope)
        XCTAssertEqual(aliases.activeRetiringSourceCount, 0)
        XCTAssertEqual(aliases.aliasCount, 0, "drain must atomically retire an eligible alias")
    }

    func testInputReadyWaitsForBarrierToReturnIdle() {
        var state = makeInputState(text: "T")
        let lifecycle = activeLifecycle(for: state)
        let scopes = activeScopes()
        let reservation = GaryxSendReservationID(rawValue: 1)
        _ = state.beginSend(
            reservationID: reservation,
            followupGeneration: 11,
            lifecycle: lifecycle,
            scopes: scopes
        )
        _ = state.releaseForCommittedNavigation(
            pendingProducers: [],
            lifecycle: lifecycle,
            scopes: scopes
        )
        XCTAssertTrue(state.commitReservation(lifecycle: lifecycle, scopes: scopes))
        XCTAssertFalse(state.inputReady, "dual terminal is insufficient while barrier is not idle")
        XCTAssertTrue(state.returnReservationToIdle(lifecycle: lifecycle, scopes: scopes))
        XCTAssertTrue(state.inputReady)
    }

    func testBarrierRejectsClientIntentIDDifferentFromSealedEnvelope() throws {
        let entry = makeEntry(id: "entry", destination: .draft("draft"), text: "T")
        var barrier = GaryxSendCommitBarrier(
            entryID: entry.id,
            scope: scope,
            payloadLifecycle: GaryxPayloadLifecycleCapture(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
        )
        let envelope = GaryxDeliveryEnvelope(
            text: "T",
            attachmentIDs: [],
            generation: 10,
            clientIntentID: "sealed-intent"
        )
        XCTAssertEqual(
            barrier.seal(
                reservationID: GaryxSendReservationID(rawValue: 1),
                envelope: envelope,
                followupGeneration: 11,
                readiness: .ready,
                quota: .init(),
                producerPhase: .live,
                lifecycle: entry.lifecycle.snapshot
            ),
            .sealed
        )
        XCTAssertNil(
            barrier.durableCommit(
                deliveryID: GaryxDeliveryRecordID(rawValue: "delivery-wrong"),
                correlationID: "correlation-wrong",
                clientIntentID: "different-intent",
                lifecycle: entry.lifecycle.snapshot
            )
        )
        XCTAssertEqual(barrier.phase, .sealed)

        let settlement = try XCTUnwrap(
            barrier.durableCommit(
                deliveryID: GaryxDeliveryRecordID(rawValue: "delivery"),
                correlationID: "correlation",
                clientIntentID: "sealed-intent",
                lifecycle: entry.lifecycle.snapshot
            )
        )
        XCTAssertEqual(settlement.deliveryRecord?.envelope?.clientIntentID, "sealed-intent")
    }

    func testAliasAdmissionNeverExceedsPersistentByteBudget() {
        var aliases = GaryxComposerAliasTable()
        XCTAssertEqual(
            aliases.establishPromotion(
                scope: scope,
                source: .draft(String(repeating: "d", count: 40_000)),
                target: .thread(String(repeating: "t", count: 40_000)),
                activeOrClosingSessions: 1
            ),
            .rejectedCapacity
        )
        XCTAssertEqual(aliases.aliasCount, 0)
        XCTAssertLessThanOrEqual(aliases.estimatedBytes, 64 * 1024)
    }

    func testDiscardResourceSettlementDoesNotTouchAnotherEntry() throws {
        var discardedEntry = makeEntry(id: "discarded", destination: .draft("discarded"))
        XCTAssertTrue(discardedEntry.beginDiscard(revision: 2))
        let unrelatedEntryID = entryID("unrelated")
        let assetID = GaryxStagedAssetID(rawValue: "unrelated-asset")
        let unrelated = makeOperation(
            entryID: unrelatedEntryID,
            operationID: "unrelated-operation",
            state: .requested,
            stagedAssetID: assetID,
            reservedBytes: 8
        )
        let barrier = GaryxSendCommitBarrier(
            entryID: discardedEntry.id,
            scope: scope,
            payloadLifecycle: GaryxPayloadLifecycleCapture(
                token: discardedEntry.lifecycle.token,
                revision: discardedEntry.lifecycle.revision
            )
        )
        var convergence = GaryxPayloadDiscardConvergence(
            lifecycle: discardedEntry.lifecycle,
            barrier: barrier,
            operations: [unrelated.context.key: unrelated],
            stagedAssetIDs: [assetID],
            reservedBytes: 8
        )

        convergence.settleResources()

        let preserved = try XCTUnwrap(convergence.operations[unrelated.context.key])
        XCTAssertEqual(preserved, unrelated)
        XCTAssertEqual(convergence.stagedAssetIDs, [assetID])
        XCTAssertEqual(convergence.reservedBytes, 8)
        XCTAssertTrue(convergence.resourcesSettled)
    }

    func testEvidenceIngressRejectsAcknowledgementBeforeTransportBoundary() {
        var record = makeDelivery(id: "delivery", correlation: "correlation")
        let id = record.id
        var records = [id: record]
        let result = GaryxDeliveryEvidenceIngress.acknowledge(
            correlationID: "correlation",
            authenticatedScope: scope,
            records: &records
        )

        XCTAssertEqual(result, .rejectedPhase)
        XCTAssertEqual(records[id]?.phase, .notDispatched)
        XCTAssertEqual(records[id]?.evidence, GaryxDeliveryEvidence.none)
        record = try! XCTUnwrap(records[id])
        XCTAssertEqual(record.envelope?.text, "T")
    }

    func testEvidenceIngressRejectsAmbiguousCorrelationWithoutChoosingArbitrarily() {
        var first = makeDelivery(id: "first", correlation: "shared")
        var second = makeDelivery(id: "second", correlation: "shared")
        XCTAssertTrue(first.markTransportAttempted())
        XCTAssertTrue(second.markTransportAttempted())
        XCTAssertTrue(second.markAmbiguous())
        let firstID = first.id
        let secondID = second.id
        var records = [firstID: first, secondID: second]

        let result = GaryxDeliveryEvidenceIngress.acknowledge(
            correlationID: "shared",
            authenticatedScope: scope,
            records: &records
        )

        XCTAssertEqual(result, .ambiguousCorrelation)
        XCTAssertEqual(records[firstID]?.phase, .transportAttempted)
        XCTAssertEqual(records[secondID]?.phase, .ambiguous)
    }

    func testGenerationResetCannotCollideWithNextHiLoAllocation() {
        var entry = makeEntry(
            id: "entry",
            destination: .draft("draft"),
            generation: 32,
            text: "draft"
        )
        var allocator = GaryxDurableHiLoAllocator(persistedHighWatermark: 32, blockSize: 32)
        let allocatedForReset = allocator.allocate()
        XCTAssertTrue(
            entry.resetGeneration(
                32,
                to: allocatedForReset,
                barrierIdle: true,
                producerLive: true
            )
        )

        XCTAssertNotEqual(
            entry.currentGeneration,
            allocator.allocate(),
            "generation reset must consume an identity from the shared durable allocator"
        )
    }

    private func entryID(_ value: String) -> GaryxComposerPayloadEntryID {
        GaryxComposerPayloadEntryID(rawValue: value)
    }

    private func activeLifecycle(for entryID: GaryxComposerPayloadEntryID) -> GaryxPayloadLifecycleSnapshot {
        GaryxPayloadLifecycleSnapshot(
            token: GaryxPayloadLifecycleToken(entryID: entryID, nonce: "token-\(entryID.rawValue)"),
            revision: 1,
            phase: .active
        )
    }

    private func activeLifecycle(
        for state: GaryxComposerInputReducerState
    ) -> GaryxPayloadLifecycleSnapshot {
        GaryxPayloadLifecycleSnapshot(
            token: state.session.payloadLifecycle.token,
            revision: state.session.payloadLifecycle.revision,
            phase: .active
        )
    }

    private func activeScopes() -> GaryxGatewayScopeRegistry {
        GaryxGatewayScopeRegistry(initialActiveScope: scope)
    }

    private func makeEntry(
        id: String,
        destination: GaryxComposerKey,
        generation: UInt64 = 10,
        text: String = ""
    ) -> GaryxComposerPayloadEntry {
        let id = entryID(id)
        return GaryxComposerPayloadEntry(
            id: id,
            scope: scope,
            destination: destination,
            lifecycleToken: GaryxPayloadLifecycleToken(entryID: id, nonce: "token-\(id.rawValue)"),
            currentGeneration: generation,
            text: text
        )
    }

    private func makeInputState(text: String) -> GaryxComposerInputReducerState {
        let id = entryID("input-entry")
        let token = GaryxPayloadLifecycleToken(entryID: id, nonce: "token-input")
        return GaryxComposerInputReducerState(
            session: GaryxComposerInputSession(
                composerKey: .draft("draft"),
                sessionID: GaryxComposerInputSessionID(rawValue: "session"),
                epoch: 1,
                scope: scope,
                payloadLifecycle: GaryxPayloadLifecycleCapture(token: token, revision: 1)
            ),
            payloadGeneration: 10,
            initialText: text
        )
    }

    private func makeOperation(
        entryID: GaryxComposerPayloadEntryID,
        operationID: String,
        state: GaryxOperationCapabilityState,
        stagedAssetID: GaryxStagedAssetID?,
        reservedBytes: Int
    ) -> GaryxOperationCapability {
        let lifecycle = activeLifecycle(for: entryID)
        let key = GaryxOperationCapabilityKey(
            scope: scope,
            entryID: entryID,
            generation: 10,
            reservationID: nil,
            branch: .followup,
            operationID: GaryxOperationID(rawValue: operationID)
        )
        return GaryxOperationCapability(
            context: GaryxScopeBoundOperationContext(
                key: key,
                clientIdentity: "client",
                configurationFingerprint: "configuration",
                payloadLifecycle: GaryxPayloadLifecycleCapture(
                    token: lifecycle.token,
                    revision: lifecycle.revision
                )
            ),
            state: state,
            stagedAssetID: stagedAssetID,
            reservedBytes: reservedBytes
        )
    }

    private func makeDelivery(id: String, correlation: String) -> GaryxDeliveryRecord {
        GaryxDeliveryRecord(
            id: GaryxDeliveryRecordID(rawValue: id),
            scope: scope,
            entryID: entryID("delivery-entry"),
            reservationID: GaryxSendReservationID(rawValue: 1),
            correlationID: correlation,
            envelope: GaryxDeliveryEnvelope(
                text: "T",
                attachmentIDs: [],
                generation: 10,
                clientIntentID: "intent-\(id)"
            )
        )
    }
}
