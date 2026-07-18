import XCTest
@testable import GaryxMobileCore

final class GaryxComposerDeliveryProtocolTests: XCTestCase {
    private let scope = GaryxGatewayScope(identity: "gateway", epoch: 1)

    func testHiLoAllocatorBatchesDurableWritesAndNeverReusesAcrossRestart() {
        var firstProcess = GaryxDurableHiLoAllocator(blockSize: 4)
        let firstValues = (0..<6).map { _ in firstProcess.allocate() }
        XCTAssertEqual(firstValues, [1, 2, 3, 4, 5, 6])
        XCTAssertEqual(firstProcess.persistedHighWatermark, 8)
        XCTAssertEqual(firstProcess.durableReservationCount, 2)

        var relaunched = GaryxDurableHiLoAllocator(
            persistedHighWatermark: firstProcess.persistedHighWatermark,
            blockSize: 4
        )
        let relaunchedValues = (0..<5).map { _ in relaunched.allocate() }
        XCTAssertEqual(relaunchedValues, [9, 10, 11, 12, 13])
        XCTAssertTrue(Set(firstValues).isDisjoint(with: Set(relaunchedValues)))
        XCTAssertEqual(relaunched.durableReservationCount, 2)
    }

    func testReservationAdmissionRequiresLedgerBeforeDescendantsAndNetwork() {
        var tracker = GaryxReservationAdmissionTracker()
        XCTAssertFalse(tracker.persistDescendant())
        XCTAssertFalse(tracker.crossNetworkBoundary())
        tracker.persistLedger()
        XCTAssertTrue(tracker.persistDescendant())
        XCTAssertTrue(tracker.persistDescendant())
        XCTAssertTrue(tracker.crossNetworkBoundary())
        XCTAssertEqual(tracker.durableDescendantCount, 2)
        XCTAssertTrue(tracker.networkAttempted)
    }

    func testReservationLedgerPublishesOutcomeAndTargetMappingTogether() {
        var ledger = makeLedger()
        XCTAssertFalse(ledger.settle(.committed, targetGeneration: 12))
        XCTAssertFalse(ledger.settle(.revoked, targetGeneration: 11))
        XCTAssertNil(ledger.terminalOutcome)
        XCTAssertTrue(ledger.settle(.revoked, targetGeneration: 12))
        XCTAssertEqual(ledger.terminalOutcome, .revoked)
        XCTAssertEqual(
            ledger.targetMapping,
            GaryxReservationTargetMapping(entryID: entryID, generation: 12)
        )
        XCTAssertFalse(ledger.settle(.revoked, targetGeneration: 13))
    }

    func testBarrierSealGatesLifecycleProducerReadinessAndQuotaWithoutAdvancing() {
        let active = makeEntry().lifecycle.snapshot
        let envelope = makeEnvelope(text: "T")

        var blockedByOperation = makeBarrier()
        XCTAssertEqual(
            blockedByOperation.seal(
                reservationID: reservation,
                envelope: envelope,
                followupGeneration: 11,
                readiness: .payloadPreparing,
                quota: .init(),
                producerPhase: .live,
                lifecycle: active
            ),
            .payloadPreparing
        )
        XCTAssertEqual(blockedByOperation.phase, .idle)

        var blockedByProducer = makeBarrier()
        XCTAssertEqual(
            blockedByProducer.seal(
                reservationID: reservation,
                envelope: envelope,
                followupGeneration: 11,
                readiness: .ready,
                quota: .init(),
                producerPhase: .finalizing,
                lifecycle: active
            ),
            .rejectedProducerPhase
        )

        var blockedByQuota = makeBarrier()
        XCTAssertEqual(
            blockedByQuota.seal(
                reservationID: reservation,
                envelope: envelope,
                followupGeneration: 11,
                readiness: .ready,
                quota: GaryxDeliveryQuota(
                    nonTerminalByScope: [scope: 64],
                    nonTerminalGlobal: 64
                ),
                producerPhase: .live,
                lifecycle: active
            ),
            .deliveryBackpressure
        )

        var lifecycle = makeEntry().lifecycle
        XCTAssertTrue(lifecycle.beginDiscard(discardRevision: 2))
        var blockedByLifecycle = makeBarrier()
        XCTAssertEqual(
            blockedByLifecycle.seal(
                reservationID: reservation,
                envelope: envelope,
                followupGeneration: 11,
                readiness: .ready,
                quota: .init(),
                producerPhase: .live,
                lifecycle: lifecycle.snapshot
            ),
            .rejectedLifecycle
        )
    }

    func testScopeSettlementAlwaysFollowsBarrierSettlementAcrossEveryPhase() {
        for scopeSettlement in [
            GaryxGatewayScopeSettlementKind.suspend,
            .revoke,
        ] {
            let scopeAction: GaryxScopeBarrierSettlementAction = scopeSettlement == .suspend
                ? .suspendScope
                : .revokeScope
            for phase in GaryxSendCommitBarrierPhase.allCases {
                if phase == .sealed {
                    XCTAssertEqual(
                        GaryxScopeBarrierSettlementPlanner.plan(
                            barrierPhase: phase,
                            scopeSettlement: scopeSettlement
                        ),
                        .awaitingSealedBarrierDecision
                    )
                    for decision in [
                        GaryxSealedBarrierSettlementDecision.durableCommit,
                        .revoke,
                    ] {
                        let terminal: GaryxScopeBarrierSettlementAction = decision == .durableCommit
                            ? .durableCommitBarrier
                            : .revokeBarrier
                        XCTAssertEqual(
                            GaryxScopeBarrierSettlementPlanner.plan(
                                barrierPhase: phase,
                                scopeSettlement: scopeSettlement,
                                sealedDecision: decision
                            ),
                            .ready([terminal, .returnBarrierToIdle, scopeAction])
                        )
                    }
                } else {
                    let expected: [GaryxScopeBarrierSettlementAction] = phase == .idle
                        ? [scopeAction]
                        : [.returnBarrierToIdle, scopeAction]
                    XCTAssertEqual(
                        GaryxScopeBarrierSettlementPlanner.plan(
                            barrierPhase: phase,
                            scopeSettlement: scopeSettlement
                        ),
                        .ready(expected)
                    )
                }
            }
        }
    }

    func testDurableCommitIsLinearizationAndPostSealPayloadStaysFollowup() throws {
        var barrier = makeBarrier()
        let lifecycle = makeEntry().lifecycle.snapshot
        let envelopeAttachment = GaryxAttachmentID(rawValue: "envelope")
        let followupAttachment = GaryxAttachmentID(rawValue: "followup")
        XCTAssertEqual(
            barrier.seal(
                reservationID: reservation,
                envelope: makeEnvelope(text: "T", attachments: [envelopeAttachment]),
                followupGeneration: 11,
                readiness: .ready,
                quota: .init(),
                producerPhase: .live,
                lifecycle: lifecycle
            ),
            .sealed
        )
        XCTAssertEqual(barrier.phase, .sealed, "seal is provisional, not linearized")
        XCTAssertTrue(barrier.replaceProvisionalText("U", lifecycle: lifecycle))
        XCTAssertTrue(
            barrier.addProvisionalAttachment(followupAttachment, lifecycle: lifecycle)
        )

        let settlement = try XCTUnwrap(
            barrier.durableCommit(
                deliveryID: deliveryID("delivery"),
                correlationID: "correlation",
                clientIntentID: "intent",
                lifecycle: lifecycle
            )
        )
        XCTAssertEqual(barrier.phase, .durableCommitted)
        XCTAssertEqual(settlement.terminalOutcome, .committed)
        XCTAssertEqual(settlement.followupGeneration, 11)
        XCTAssertEqual(settlement.followupText, "U")
        XCTAssertEqual(settlement.followupAttachmentIDs, [followupAttachment])
        XCTAssertEqual(settlement.deliveryRecord?.envelope?.text, "T")
        XCTAssertEqual(settlement.deliveryRecord?.envelope?.attachmentIDs, [envelopeAttachment])
        XCTAssertFalse(
            settlement.deliveryRecord?.envelope?.attachmentIDs.contains(followupAttachment) == true,
            "S1 is immutable after durable commit"
        )
    }

    func testRevokedBarrierConsumesGenerationAndMergesTPlusUAtGPlusTwo() throws {
        var barrier = makeBarrier()
        let lifecycle = makeEntry().lifecycle.snapshot
        let oldAttachment = GaryxAttachmentID(rawValue: "old")
        let newAttachment = GaryxAttachmentID(rawValue: "new")
        _ = barrier.seal(
            reservationID: reservation,
            envelope: makeEnvelope(text: "T", attachments: [oldAttachment]),
            followupGeneration: 11,
            readiness: .ready,
            quota: .init(),
            producerPhase: .live,
            lifecycle: lifecycle
        )
        XCTAssertTrue(barrier.replaceProvisionalText("U", lifecycle: lifecycle))
        XCTAssertTrue(barrier.addProvisionalAttachment(newAttachment, lifecycle: lifecycle))
        let settlement = try XCTUnwrap(
            barrier.revoke(mergeGeneration: 12, lifecycle: lifecycle)
        )
        XCTAssertEqual(settlement.terminalOutcome, .revoked)
        XCTAssertEqual(settlement.followupGeneration, 12)
        XCTAssertEqual(settlement.followupText, "TU")
        XCTAssertEqual(settlement.followupAttachmentIDs, [oldAttachment, newAttachment])
        XCTAssertNil(settlement.deliveryRecord)
    }

    func testBarrierCanReturnIdleWhileMultipleDeliveryRecordsProgressIndependently() throws {
        var firstBarrier = makeBarrier()
        let lifecycle = makeEntry().lifecycle.snapshot
        _ = firstBarrier.seal(
            reservationID: reservation,
            envelope: makeEnvelope(text: "S1"),
            followupGeneration: 11,
            readiness: .ready,
            quota: .init(),
            producerPhase: .live,
            lifecycle: lifecycle
        )
        var first = try XCTUnwrap(
            firstBarrier.durableCommit(
                deliveryID: deliveryID("s1"),
                correlationID: "c1",
                clientIntentID: "intent",
                lifecycle: lifecycle
            )?.deliveryRecord
        )
        firstBarrier.returnToIdle()
        XCTAssertEqual(firstBarrier.phase, .idle)
        XCTAssertTrue(first.markTransportAttempted())
        XCTAssertTrue(first.markAmbiguous())

        let secondReservation = GaryxSendReservationID(rawValue: 2)
        _ = firstBarrier.seal(
            reservationID: secondReservation,
            envelope: GaryxDeliveryEnvelope(
                text: "S2",
                attachmentIDs: [],
                generation: 11,
                clientIntentID: "i2"
            ),
            followupGeneration: 12,
            readiness: .ready,
            quota: .init(),
            producerPhase: .live,
            lifecycle: lifecycle
        )
        var second = try XCTUnwrap(
            firstBarrier.durableCommit(
                deliveryID: deliveryID("s2"),
                correlationID: "c2",
                clientIntentID: "i2",
                lifecycle: lifecycle
            )?.deliveryRecord
        )
        XCTAssertTrue(second.markTransportAttempted())
        second.recordServerAcknowledgement()
        XCTAssertEqual(first.phase, .ambiguous)
        XCTAssertEqual(second.phase, .acknowledged)
    }

    func testDeliveryRecordTransportBoundaryAndAmbiguousUserExits() throws {
        var retryable = makeDelivery("retry")
        XCTAssertEqual(retryable.phase, .notDispatched)
        XCTAssertTrue(retryable.markTransportAttempted())
        XCTAssertFalse(retryable.markTransportAttempted(), "attempt boundary is exactly once")
        XCTAssertTrue(retryable.markAmbiguous())
        var conflicts = makeConflictSet("restore")
        let beforeRecord = retryable
        XCTAssertEqual(
            GaryxDeliveryDraftRecoveryReducer.restore(
                record: &retryable,
                conflictSet: &conflicts,
                candidate: conflictCandidate("restore"),
                membershipDurabilityAvailable: false
            ),
            .rejectedConflictDurability
        )
        XCTAssertEqual(retryable, beforeRecord)
        XCTAssertTrue(conflicts.candidates.isEmpty)
        let recovery = GaryxDeliveryDraftRecoveryReducer.restore(
            record: &retryable,
            conflictSet: &conflicts,
            candidate: conflictCandidate("restore"),
            membershipDurabilityAvailable: true
        )
        guard case .restored(let restored) = recovery else {
            return XCTFail("expected restored envelope, got \(recovery)")
        }
        XCTAssertEqual(restored.text, "message")
        XCTAssertEqual(conflicts.candidates.map(\.entryID), [conflictCandidate("restore").entryID])
        XCTAssertEqual(retryable.phase, .abandoned)
        XCTAssertEqual(retryable.userDisposition, .restoredToDraft)
        XCTAssertNil(retryable.envelope)

        var duplicate = makeDelivery("duplicate")
        _ = duplicate.markTransportAttempted()
        _ = duplicate.markAmbiguous()
        let newID = deliveryID("new")
        let duplicateEnvelope = try XCTUnwrap(
            duplicate.resendAsDuplicate(
                newRecordID: newID,
                newClientIntentID: "duplicate-intent"
            )
        )
        XCTAssertEqual(duplicateEnvelope.text, "message")
        XCTAssertEqual(duplicateEnvelope.clientIntentID, "duplicate-intent")
        XCTAssertEqual(duplicate.phase, .supersededByDuplicate)
        XCTAssertEqual(duplicate.duplicateRecordID, newID)
        XCTAssertEqual(duplicate.userDisposition, .resentAsDuplicate)
    }

    func testDeliveryEvidenceAndUserDispositionAreOrthogonalInBothOrders() {
        for evidenceFirst in [false, true] {
            var record = makeDelivery(evidenceFirst ? "evidence-first" : "disposition-first")
            var conflicts = makeConflictSet(evidenceFirst ? "evidence" : "disposition")
            let candidate = conflictCandidate(evidenceFirst ? "evidence" : "disposition")
            _ = record.markTransportAttempted()
            _ = record.markAmbiguous()
            if evidenceFirst {
                record.recordServerAcknowledgement()
                XCTAssertEqual(record.evidence, .serverAcknowledged)
                XCTAssertEqual(
                    GaryxDeliveryDraftRecoveryReducer.restore(
                        record: &record,
                        conflictSet: &conflicts,
                        candidate: candidate,
                        membershipDurabilityAvailable: true
                    ),
                    .rejectedNotAmbiguous
                )
            } else {
                guard case .restored = GaryxDeliveryDraftRecoveryReducer.restore(
                    record: &record,
                    conflictSet: &conflicts,
                    candidate: candidate,
                    membershipDurabilityAvailable: true
                ) else {
                    return XCTFail("expected restore before evidence")
                }
                record.recordServerAcknowledgement()
                XCTAssertEqual(record.userDisposition, .restoredToDraft)
                XCTAssertEqual(record.evidence, .serverAcknowledged)
                XCTAssertEqual(record.phase, .abandoned)
            }
        }
    }

    func testEvidenceIngressAuthenticatesSourceAndCannotCarryDomainContent() {
        let id = deliveryID("record")
        var records = [id: makeDelivery("record", correlationID: "correlation")]
        XCTAssertTrue(records[id]?.markTransportAttempted() == true)
        let wrongScope = GaryxGatewayScope(identity: "other", epoch: 1)
        XCTAssertEqual(
            GaryxDeliveryEvidenceIngress.acknowledge(
                correlationID: "correlation",
                authenticatedScope: wrongScope,
                records: &records
            ),
            .rejectedAuthenticationSource
        )
        XCTAssertEqual(
            GaryxDeliveryEvidenceIngress.acknowledge(
                correlationID: "unknown",
                authenticatedScope: scope,
                records: &records
            ),
            .unknownCorrelation
        )
        XCTAssertEqual(
            GaryxDeliveryEvidenceIngress.acknowledge(
                correlationID: "correlation",
                authenticatedScope: scope,
                records: &records
            ),
            .updated(id)
        )
        XCTAssertEqual(records[id]?.evidence, .serverAcknowledged)
        XCTAssertNil(records[id]?.envelope, "ingress keeps only bounded evidence")
    }

    func testEvidenceIngressRemainsOrthogonalAfterAmbiguousUserDisposition() throws {
        for restoredToDraft in [false, true] {
            let suffix = restoredToDraft ? "restored" : "duplicate"
            let id = deliveryID(suffix)
            let correlationID = "correlation-\(suffix)"
            var record = makeDelivery(suffix, correlationID: correlationID)
            XCTAssertTrue(record.markTransportAttempted())
            XCTAssertTrue(record.markAmbiguous())

            if restoredToDraft {
                var conflict = makeConflictSet(suffix)
                guard case .restored = GaryxDeliveryDraftRecoveryReducer.restore(
                    record: &record,
                    conflictSet: &conflict,
                    candidate: conflictCandidate(suffix),
                    membershipDurabilityAvailable: true
                ) else {
                    return XCTFail("expected restored-to-draft disposition")
                }
            } else {
                XCTAssertNotNil(
                    record.resendAsDuplicate(
                        newRecordID: deliveryID("new-\(suffix)"),
                        newClientIntentID: "intent-\(suffix)"
                    )
                )
            }
            let terminalPhase = record.phase
            let terminalDisposition = record.userDisposition
            var records = [id: record]

            XCTAssertEqual(
                GaryxDeliveryEvidenceIngress.acknowledge(
                    correlationID: correlationID,
                    authenticatedScope: scope,
                    records: &records
                ),
                .updated(id),
                "late evidence must remain orthogonal after user disposition"
            )
            XCTAssertEqual(records[id]?.phase, terminalPhase)
            XCTAssertEqual(records[id]?.userDisposition, terminalDisposition)
            XCTAssertEqual(records[id]?.evidence, .serverAcknowledged)
        }
    }

    func testQuotaBoundariesAreInclusiveAndNeverRequireEviction() {
        let envelope = makeEnvelope(text: "1234")
        let within = GaryxDeliveryQuota(
            nonTerminalByScope: [scope: 63],
            nonTerminalGlobal: 255,
            payloadBytesUsed: 6,
            payloadByteLimit: 10
        )
        XCTAssertTrue(within.canSeal(scope: scope, envelopeBytes: envelope.estimatedBytes))
        let scopeFull = GaryxDeliveryQuota(
            nonTerminalByScope: [scope: 64],
            nonTerminalGlobal: 64
        )
        XCTAssertFalse(scopeFull.canSeal(scope: scope, envelopeBytes: 0))
        let globalFull = GaryxDeliveryQuota(nonTerminalGlobal: 256)
        XCTAssertFalse(globalFull.canSeal(scope: scope, envelopeBytes: 0))
    }

    func testOfflineAmbiguousQuotaSurvivesRelaunchAndReclaimsToSteadyState() throws {
        let scopes = (0..<4).map {
            GaryxGatewayScope(identity: "gateway-\($0)", epoch: 1)
        }
        var records: [GaryxDeliveryRecord] = []
        for (scopeIndex, recordScope) in scopes.enumerated() {
            for row in 0..<GaryxDeliveryQuota.perScopeRecordLimit {
                var record = GaryxDeliveryRecord(
                    id: GaryxDeliveryRecordID(rawValue: "delivery-\(scopeIndex)-\(row)"),
                    scope: recordScope,
                    entryID: GaryxComposerPayloadEntryID(rawValue: "entry-\(scopeIndex)"),
                    reservationID: GaryxSendReservationID(rawValue: UInt64(row + 1)),
                    correlationID: "correlation-\(scopeIndex)-\(row)",
                    envelope: GaryxDeliveryEnvelope(
                        text: "offline-message",
                        attachmentIDs: [],
                        generation: UInt64(row + 1),
                        clientIntentID: "intent-\(scopeIndex)-\(row)"
                    )
                )
                XCTAssertTrue(record.markTransportAttempted())
                XCTAssertTrue(record.markAmbiguous())
                records.append(record)
            }
        }
        XCTAssertEqual(records.count, GaryxDeliveryQuota.globalRecordLimit)

        let relaunched = try JSONDecoder().decode(
            [GaryxDeliveryRecord].self,
            from: JSONEncoder().encode(records)
        )
        var quota = GaryxDeliveryQuota(rebuilding: relaunched)
        XCTAssertEqual(quota.nonTerminalGlobal, GaryxDeliveryQuota.globalRecordLimit)
        XCTAssertEqual(quota.nonTerminalByScope.values.sorted(), [64, 64, 64, 64])
        for recordScope in scopes {
            XCTAssertFalse(quota.canSeal(scope: recordScope, envelopeBytes: 0))
        }

        var reclaimed = relaunched
        reclaimed[0].recordServerAcknowledgement()
        quota = GaryxDeliveryQuota(rebuilding: reclaimed)
        XCTAssertEqual(quota.nonTerminalGlobal, GaryxDeliveryQuota.globalRecordLimit - 1)
        XCTAssertTrue(quota.canSeal(scope: scopes[0], envelopeBytes: 0))
        XCTAssertFalse(quota.canSeal(scope: scopes[1], envelopeBytes: 0))

        for index in reclaimed.indices {
            reclaimed[index].recordServerAcknowledgement()
        }
        let steadyRelaunch = try JSONDecoder().decode(
            [GaryxDeliveryRecord].self,
            from: JSONEncoder().encode(reclaimed)
        )
        quota = GaryxDeliveryQuota(rebuilding: steadyRelaunch)
        XCTAssertEqual(quota.nonTerminalGlobal, 0)
        XCTAssertTrue(quota.nonTerminalByScope.isEmpty)
        XCTAssertEqual(quota.payloadBytesUsed, 0)
        XCTAssertTrue(quota.canSeal(scope: scopes[0], envelopeBytes: 1))
    }

    func testCreateResponseLossIsExplicitlyAmbiguousAtEveryUnacknowledgedStage() {
        var create = GaryxCreateDeliveryState(scope: scope, createIntentID: "create-intent")
        create.responseLost()
        XCTAssertEqual(create.phase, .ambiguous)
        XCTAssertEqual(create.ambiguousAfter, .createPending)

        var afterCreate = GaryxCreateDeliveryState(scope: scope, createIntentID: "after-create")
        afterCreate.created(threadID: "thread")
        afterCreate.responseLost()
        XCTAssertEqual(afterCreate.phase, .ambiguous)
        XCTAssertEqual(afterCreate.ambiguousAfter, .threadCreated)
        XCTAssertEqual(afterCreate.threadID, "thread")

        var afterBinding = GaryxCreateDeliveryState(scope: scope, createIntentID: "after-bind")
        afterBinding.created(threadID: "thread")
        afterBinding.bound()
        afterBinding.responseLost()
        XCTAssertEqual(afterBinding.ambiguousAfter, .bindingCompleted)

        var lateAcknowledgement = GaryxCreateDeliveryState(
            scope: scope,
            createIntentID: "late-ack"
        )
        lateAcknowledgement.created(threadID: "thread")
        lateAcknowledgement.chatStartAttempted()
        lateAcknowledgement.responseLost()
        XCTAssertEqual(lateAcknowledgement.ambiguousAfter, .chatStartAttempted)
        lateAcknowledgement.acknowledged()
        XCTAssertEqual(lateAcknowledgement.phase, .acknowledged)
        XCTAssertNil(lateAcknowledgement.ambiguousAfter)

        var chat = GaryxCreateDeliveryState(scope: scope, createIntentID: "chat")
        chat.created(threadID: "thread")
        chat.bound()
        chat.chatStartAttempted()
        chat.acknowledged()
        chat.responseLost()
        XCTAssertEqual(chat.phase, .acknowledged)

        var restore = create
        XCTAssertTrue(restore.restoreToDraft())
        XCTAssertFalse(restore.restoreToDraft())
        XCTAssertEqual(restore.userDisposition, .restoredToDraft)

        var rebuild = afterCreate
        let duplicateRisk = rebuild.rebuildWithDuplicateRisk(newCreateIntentID: "rebuild")
        XCTAssertEqual(rebuild.userDisposition, .rebuildMayCreateDuplicateThread)
        XCTAssertEqual(duplicateRisk?.scope, scope)
        XCTAssertEqual(duplicateRisk?.createIntentID, "rebuild")
        XCTAssertEqual(duplicateRisk?.phase, .createPending)
    }

    func testDiscardDeliverySettlementIsIndependentOfBarrierPhaseAndCASWins() {
        let phases: [GaryxDeliveryRecordPhase] = [
            .notDispatched, .transportAttempted, .ambiguous, .acknowledged,
        ]
        for phase in phases {
            var record = makeDelivery("record-\(phase)")
            if phase != .notDispatched { _ = record.markTransportAttempted() }
            if phase == .ambiguous { _ = record.markAmbiguous() }
            if phase == .acknowledged { record.recordServerAcknowledgement() }
            record.settleForDiscard()
            let expected: GaryxDeliveryRecordPhase = switch phase {
            case .notDispatched: .cancelledByDiscard
            case .transportAttempted, .ambiguous: .evidence
            case .acknowledged: .terminalEvidence
            default: fatalError("unexpected phase")
            }
            XCTAssertEqual(record.phase, expected)
            XCTAssertNil(record.envelope)
            XCTAssertFalse(record.markTransportAttempted(), "discard CAS already won")
        }
    }

    func testScopeRevokeSettlesEveryDeliveryPhaseWithoutOverwritingPriorUserExit() {
        for phase in [
            GaryxDeliveryRecordPhase.notDispatched,
            .transportAttempted,
            .ambiguous,
            .acknowledged,
        ] {
            var record = makeDelivery("scope-\(phase.rawValue)")
            if phase != .notDispatched { XCTAssertTrue(record.markTransportAttempted()) }
            if phase == .ambiguous { XCTAssertTrue(record.markAmbiguous()) }
            if phase == .acknowledged { record.recordServerAcknowledgement() }
            record.settleForScopeRevoke()
            let expected: GaryxDeliveryRecordPhase = switch phase {
            case .notDispatched: .cancelledByDiscard
            case .transportAttempted, .ambiguous: .evidence
            case .acknowledged: .terminalEvidence
            default: fatalError("unexpected phase")
            }
            XCTAssertEqual(record.phase, expected)
            XCTAssertEqual(record.userDisposition, .scopeRevoked)
            XCTAssertNil(record.envelope)
        }

        var restored = makeDelivery("restored")
        XCTAssertTrue(restored.markTransportAttempted())
        XCTAssertTrue(restored.markAmbiguous())
        var conflicts = makeConflictSet("scope-restored")
        guard case .restored = GaryxDeliveryDraftRecoveryReducer.restore(
            record: &restored,
            conflictSet: &conflicts,
            candidate: conflictCandidate("scope-restored"),
            membershipDurabilityAvailable: true
        ) else {
            return XCTFail("expected restore")
        }
        restored.settleForScopeRevoke()
        XCTAssertEqual(restored.phase, .abandoned)
        XCTAssertEqual(restored.userDisposition, .restoredToDraft)

        var duplicated = makeDelivery("duplicated")
        XCTAssertTrue(duplicated.markTransportAttempted())
        XCTAssertTrue(duplicated.markAmbiguous())
        XCTAssertNotNil(
            duplicated.resendAsDuplicate(
                newRecordID: deliveryID("copy"),
                newClientIntentID: "copy-intent"
            )
        )
        duplicated.settleForScopeRevoke()
        XCTAssertEqual(duplicated.phase, .supersededByDuplicate)
        XCTAssertEqual(duplicated.userDisposition, .resentAsDuplicate)
    }

    func testDiscardSessionReducerTableForIdleAndSealedReservations() throws {
        for barrierSealed in [false, true] {
            for phase in [
                GaryxSessionDescendantPhase.live,
                .finalizing,
                .closePendingAck,
                .retired,
            ] {
                let session = makeSession("session", epoch: 1, key: .draft("D"), phase: phase)
                var convergence = makeConvergence(
                    barrierSealed: barrierSealed,
                    sessions: [session.key: session]
                )
                convergence.settleReservation()
                convergence.settleSessions()
                XCTAssertTrue(convergence.descendantsEmpty)
                if phase == .retired {
                    XCTAssertTrue(convergence.tombstones.isEmpty)
                } else {
                    let tombstone = try XCTUnwrap(convergence.tombstones.values.first)
                    XCTAssertEqual(
                        tombstone.disposition,
                        phase == .closePendingAck
                            ? .closePendingAckConverted
                            : .finalizerTerminated
                    )
                }
            }
        }
    }

    func testV41PendingAckThenActualPromotionThenLiveSessionDiscardSettlesBoth() throws {
        var entry = makeEntry()
        let stableToken = entry.lifecycle.token
        var payloadStore = GaryxComposerPayloadStore()
        XCTAssertTrue(payloadStore.insert(entry))
        let first = makeSession(
            "s1",
            epoch: 1,
            key: .draft("D"),
            phase: .closePendingAck
        )
        XCTAssertEqual(first.key.token, stableToken)
        XCTAssertTrue(
            payloadStore.promote(
                entryID: entry.id,
                scope: scope,
                to: .thread("T")
            )
        )
        XCTAssertEqual(
            payloadStore.entry(entry.id, scope: scope)?.destination,
            .thread("T")
        )
        XCTAssertEqual(
            payloadStore.entry(entry.id, scope: scope)?.lifecycle.token,
            stableToken
        )
        var aliases = GaryxComposerAliasTable()
        XCTAssertEqual(
            aliases.establishPromotion(
                scope: scope,
                source: .draft("D"),
                target: .thread("T"),
                activeOrClosingSessions: 1,
                pendingCloseAcknowledgements: 1
            ),
            .established
        )
        let second = makeSession(
            "s2",
            epoch: 2,
            key: .thread("T"),
            phase: .live
        )
        XCTAssertEqual(second.key.token, stableToken)

        entry = try XCTUnwrap(payloadStore.entry(entry.id, scope: scope))
        XCTAssertTrue(entry.beginDiscard(revision: 101))
        payloadStore.update(entry)
        var convergence = GaryxPayloadDiscardConvergence(
            lifecycle: entry.lifecycle,
            barrier: makeBarrier(),
            sessions: [first.key: first, second.key: second]
        )
        convergence.settleDeliveries()
        convergence.settleReservation()
        convergence.settleSessions()
        convergence.settleResources()
        XCTAssertEqual(convergence.tombstones.count, 2)
        XCTAssertEqual(
            Set(convergence.tombstones.keys.map(\.sessionID)),
            [first.key.sessionID, second.key.sessionID]
        )
        XCTAssertTrue(convergence.descendantsEmpty)
        XCTAssertTrue(convergence.finishToken())
        XCTAssertEqual(convergence.lifecycle.phase, .discarded)
        XCTAssertEqual(
            convergence.receiveLateCloseAcknowledgement(
                sessionID: first.key.sessionID,
                epoch: first.key.epoch
            ),
            .rejectedTombstoned
        )
        XCTAssertTrue(aliases.markDrained(source: .draft("D"), scope: scope))
        XCTAssertEqual(aliases.aliasCount, 0)
        XCTAssertEqual(aliases.activeRetiringSourceCount, 0)

        let encoded = try JSONEncoder().encode(convergence.tombstones)
        let text = String(decoding: encoded, as: UTF8.self)
        XCTAssertFalse(text.localizedCaseInsensitiveContains("attachment"))
        XCTAssertFalse(text.localizedCaseInsensitiveContains("path"))
        XCTAssertFalse(text.contains("draft payload text"))
    }

    func testDiscardThreeComponentsConvergeInAnyOrderAndTokenFinishesLast() {
        let orders: [[Int]] = [
            [0, 1, 2, 3],
            [2, 0, 3, 1],
            [1, 3, 2, 0],
        ]
        for order in orders {
            let session = makeSession("session", epoch: 1, key: .draft("D"), phase: .live)
            var delivery = makeDelivery("delivery")
            _ = delivery.markTransportAttempted()
            _ = delivery.markAmbiguous()
            let operation = makeFailedRetryableOperation()
            var convergence = makeConvergence(
                barrierSealed: true,
                sessions: [session.key: session],
                deliveries: [delivery.id: delivery],
                operations: [operation.context.key: operation]
            )

            XCTAssertFalse(convergence.finishToken())
            for component in order {
                switch component {
                case 0: convergence.settleDeliveries()
                case 1: convergence.settleReservation()
                case 2: convergence.settleSessions()
                case 3: convergence.settleResources()
                default: XCTFail("unknown component")
                }
            }
            XCTAssertTrue(convergence.finishToken(), "order=\(order)")
            XCTAssertEqual(convergence.lifecycle.phase, .discarded)
            XCTAssertTrue(convergence.descendantsEmpty)
            XCTAssertTrue(convergence.deliveriesSettled)
            XCTAssertTrue(convergence.resourcesSettled)
            XCTAssertEqual(convergence.reservedBytes, 0)
            XCTAssertTrue(convergence.stagedAssetIDs.isEmpty)
            XCTAssertEqual(
                convergence.operations[operation.context.key]?.state,
                .cancelled
            )
        }
    }

    func testDiscardArchivesFeedbackAndReleasesStableAttachmentLineage() {
        var entry = makeEntry()
        XCTAssertTrue(entry.beginDiscard(revision: 100))
        let lineageID = GaryxAttachmentLineageID(rawValue: "lineage")
        let feedbackID = GaryxFeedbackID(rawValue: "feedback")
        let feedback = GaryxOperationFeedback(
            id: feedbackID,
            scope: scope,
            entryID: entryID,
            operationID: GaryxOperationID(rawValue: "failed"),
            lineageID: lineageID,
            kind: .uploadTerminal
        )
        let lineage = GaryxAttachmentLineageTombstone(
            id: lineageID,
            scope: scope,
            entryID: entryID,
            attachmentSlotID: GaryxAttachmentID(rawValue: "stable-slot"),
            failedOperationID: GaryxOperationID(rawValue: "failed"),
            feedbackID: feedbackID,
            payloadLifecycle: GaryxPayloadLifecycleCapture(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision - 1
            )
        )
        var convergence = GaryxPayloadDiscardConvergence(
            lifecycle: entry.lifecycle,
            barrier: makeBarrier(),
            feedback: [feedbackID: feedback],
            attachmentLineages: [lineageID: lineage]
        )

        convergence.settleDeliveries()
        convergence.settleReservation()
        convergence.settleSessions()
        convergence.settleResources()
        XCTAssertEqual(convergence.feedback[feedbackID]?.phase, .archived)
        XCTAssertEqual(convergence.attachmentLineages[lineageID]?.phase, .released)
        XCTAssertTrue(convergence.finishToken())
    }

    func testClosePendingAckConvertsToTombstoneAndLateAckIsRejectedThenGCs() {
        let session = makeSession(
            "session",
            epoch: 1,
            key: .draft("D"),
            phase: .closePendingAck
        )
        var convergence = makeConvergence(sessions: [session.key: session])
        convergence.settleDeliveries()
        convergence.settleReservation()
        convergence.settleSessions()
        convergence.settleResources()
        XCTAssertTrue(convergence.finishToken())
        XCTAssertEqual(
            convergence.receiveLateCloseAcknowledgement(
                sessionID: session.key.sessionID,
                epoch: session.key.epoch
            ),
            .rejectedTombstoned
        )
        XCTAssertTrue(convergence.garbageCollectTombstonesIfEligible())
        XCTAssertEqual(
            convergence.receiveLateCloseAcknowledgement(
                sessionID: session.key.sessionID,
                epoch: session.key.epoch
            ),
            .rejectedUnknownToken
        )
    }

    func testDiscardTombstoneChurnStaysBoundedAfterEligibleGC() {
        var retainedTombstones = 0
        for index in 0..<500 {
            let session = makeSession(
                "session-\(index)",
                epoch: UInt64(index + 1),
                key: .draft("D-\(index)"),
                phase: .finalizing
            )
            var convergence = makeConvergence(sessions: [session.key: session])
            convergence.settleDeliveries()
            convergence.settleReservation()
            convergence.settleSessions()
            convergence.settleResources()
            XCTAssertTrue(convergence.finishToken())
            XCTAssertTrue(convergence.garbageCollectTombstonesIfEligible())
            retainedTombstones += convergence.tombstones.count
        }
        XCTAssertEqual(retainedTombstones, 0)
    }

    private var entryID: GaryxComposerPayloadEntryID {
        GaryxComposerPayloadEntryID(rawValue: "entry")
    }

    private var reservation: GaryxSendReservationID {
        GaryxSendReservationID(rawValue: 1)
    }

    private func makeEntry() -> GaryxComposerPayloadEntry {
        GaryxComposerPayloadEntry(
            id: entryID,
            scope: scope,
            destination: .draft("D"),
            lifecycleToken: GaryxPayloadLifecycleToken(entryID: entryID, nonce: "token"),
            currentGeneration: 10,
            text: "T"
        )
    }

    private func makeBarrier() -> GaryxSendCommitBarrier {
        let entry = makeEntry()
        return GaryxSendCommitBarrier(
            entryID: entry.id,
            scope: entry.scope,
            payloadLifecycle: GaryxPayloadLifecycleCapture(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
        )
    }

    private func makeEnvelope(
        text: String,
        attachments: [GaryxAttachmentID] = []
    ) -> GaryxDeliveryEnvelope {
        GaryxDeliveryEnvelope(
            text: text,
            attachmentIDs: attachments,
            generation: 10,
            clientIntentID: "intent"
        )
    }

    private func makeLedger() -> GaryxProvisionalReservationLedger {
        GaryxProvisionalReservationLedger(
            key: GaryxReservationLedgerKey(
                scope: scope,
                entryID: entryID,
                reservationID: reservation
            ),
            envelopeGeneration: 10,
            followupGeneration: 11
        )
    }

    private func deliveryID(_ value: String) -> GaryxDeliveryRecordID {
        GaryxDeliveryRecordID(rawValue: value)
    }

    private func makeConflictSet(_ value: String) -> GaryxPayloadConflictSet {
        GaryxPayloadConflictSet(
            id: GaryxPayloadConflictSetID(rawValue: "conflict-\(value)"),
            scope: scope
        )
    }

    private func conflictCandidate(_ value: String) -> GaryxPayloadConflictCandidate {
        GaryxPayloadConflictCandidate(
            entryID: GaryxComposerPayloadEntryID(rawValue: "candidate-\(value)"),
            label: "Recovered draft"
        )
    }

    private func makeDelivery(
        _ value: String,
        correlationID: String? = nil
    ) -> GaryxDeliveryRecord {
        GaryxDeliveryRecord(
            id: deliveryID(value),
            scope: scope,
            entryID: entryID,
            reservationID: reservation,
            correlationID: correlationID ?? "correlation-\(value)",
            envelope: GaryxDeliveryEnvelope(
                text: "message",
                attachmentIDs: [],
                generation: 10,
                clientIntentID: "intent"
            )
        )
    }

    private func makeSession(
        _ id: String,
        epoch: UInt64,
        key: GaryxComposerKey,
        phase: GaryxSessionDescendantPhase
    ) -> GaryxSessionDescendant {
        let token = makeEntry().lifecycle.token
        return GaryxSessionDescendant(
            key: GaryxSessionDescendantKey(
                token: token,
                sessionID: GaryxComposerInputSessionID(rawValue: id),
                epoch: epoch
            ),
            composerKey: key,
            phase: phase,
            finalSequence: phase == .live ? nil : 4
        )
    }

    private func makeFailedRetryableOperation() -> GaryxOperationCapability {
        let entry = makeEntry()
        let key = GaryxOperationCapabilityKey(
            scope: scope,
            entryID: entryID,
            generation: 10,
            reservationID: nil,
            branch: .followup,
            operationID: GaryxOperationID(rawValue: "operation")
        )
        return GaryxOperationCapability(
            context: GaryxScopeBoundOperationContext(
                key: key,
                clientIdentity: "client",
                configurationFingerprint: "config",
                payloadLifecycle: GaryxPayloadLifecycleCapture(
                    token: entry.lifecycle.token,
                    revision: entry.lifecycle.revision
                )
            ),
            state: .failedRetryable,
            stagedAssetID: GaryxStagedAssetID(rawValue: "asset"),
            reservedBytes: 100,
            uploadAttempted: true
        )
    }

    private func makeConvergence(
        barrierSealed: Bool = false,
        sessions: [GaryxSessionDescendantKey: GaryxSessionDescendant] = [:],
        deliveries: [GaryxDeliveryRecordID: GaryxDeliveryRecord] = [:],
        operations: [GaryxOperationCapabilityKey: GaryxOperationCapability] = [:]
    ) -> GaryxPayloadDiscardConvergence {
        var entry = makeEntry()
        XCTAssertTrue(entry.beginDiscard(revision: 99))
        var barrier = makeBarrier()
        if barrierSealed {
            let activeBeforeDiscard = GaryxPayloadLifecycleSnapshot(
                token: barrier.payloadLifecycle.token,
                revision: barrier.payloadLifecycle.revision,
                phase: .active
            )
            _ = barrier.seal(
                reservationID: reservation,
                envelope: makeEnvelope(text: "T"),
                followupGeneration: 11,
                readiness: .ready,
                quota: .init(),
                producerPhase: .live,
                lifecycle: activeBeforeDiscard
            )
        }
        return GaryxPayloadDiscardConvergence(
            lifecycle: entry.lifecycle,
            barrier: barrier,
            sessions: sessions,
            deliveries: deliveries,
            operations: operations,
            stagedAssetIDs: operations.values.compactMap(\.stagedAssetID).reduce(into: Set()) {
                $0.insert($1)
            },
            reservedBytes: operations.values.reduce(0) { $0 + $1.reservedBytes }
        )
    }
}
