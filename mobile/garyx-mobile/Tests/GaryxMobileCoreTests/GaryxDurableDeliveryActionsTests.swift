import XCTest
@testable import GaryxMobileCore

final class GaryxDurableDeliveryActionsTests: XCTestCase {
    private let scope = GaryxGatewayScope(identity: "test-gateway", epoch: 4)

    func testRestoreExitPublishesConflictWithoutOverwritingFollowupThenResolvesAtomically() async throws {
        let fixture = try await makeAmbiguousFixture()
        var snapshot = try await fixture.store.load()
        let recoveryGeneration = try await fixture.store.allocatePayloadGeneration()
        snapshot = try await fixture.store.load()
        let recoveredID = GaryxComposerPayloadEntryID(rawValue: "recovered-entry")
        let conflictID = GaryxPayloadConflictSetID(rawValue: "delivery-conflict")
        let plan = try XCTUnwrap(
            GaryxDeliveryDraftRecoveryPlanner.plan(
                snapshot: snapshot,
                deliveryID: fixture.deliveryID,
                recoveredEntryID: recoveredID,
                recoveredLifecycleNonce: "recovered-token",
                recoveredGeneration: recoveryGeneration,
                conflictSetID: conflictID
            )
        )
        snapshot = try await fixture.store.commit(plan.transaction)

        XCTAssertEqual(snapshot.deliveries[fixture.deliveryID]?.phase, .abandoned)
        XCTAssertEqual(
            snapshot.deliveries[fixture.deliveryID]?.userDisposition,
            .restoredToDraft
        )
        XCTAssertEqual(
            snapshot.payloadStore.entry(fixture.entryID, scope: scope)?.currentText,
            "live follow-up"
        )
        XCTAssertEqual(
            snapshot.payloadStore.entry(recoveredID, scope: scope)?.currentText,
            "sealed message"
        )
        XCTAssertEqual(snapshot.conflicts[conflictID]?.candidates.count, 2)

        let replacementGeneration = try await fixture.store.allocatePayloadGeneration()
        snapshot = try await fixture.store.load()
        let resolution = try XCTUnwrap(
            GaryxRecoveredDraftResolutionPlanner.useRecoveredDraft(
                snapshot: snapshot,
                conflictSetID: conflictID,
                hostEntryID: fixture.entryID,
                recoveredEntryID: recoveredID,
                replacementGeneration: replacementGeneration
            )
        )
        snapshot = try await fixture.store.commit(resolution.transaction)
        let host = try XCTUnwrap(snapshot.payloadStore.entry(fixture.entryID, scope: scope))
        XCTAssertEqual(host.currentText, "sealed message")
        XCTAssertEqual(host.attachments.values.first?.uploadedPath, "prompt/file.png")
        XCTAssertNil(snapshot.payloadStore.entry(recoveredID, scope: scope))
        XCTAssertNil(snapshot.conflicts[conflictID])
    }

    func testDuplicateExitCreatesNewIntentAndLateEvidenceOnlyClaimsOriginal() async throws {
        let fixture = try await makeAmbiguousFixture()
        let snapshot = try await fixture.store.load()
        let duplicateID = GaryxDeliveryRecordID(rawValue: "duplicate-delivery")
        let plan = try XCTUnwrap(
            GaryxDeliveryDuplicateResendPlanner.plan(
                snapshot: snapshot,
                deliveryID: fixture.deliveryID,
                newDeliveryID: duplicateID,
                newClientIntentID: "copy-intent"
            )
        )
        var committed = try await fixture.store.commit(plan.transaction)
        XCTAssertEqual(committed.deliveries[fixture.deliveryID]?.phase, .supersededByDuplicate)
        XCTAssertEqual(committed.deliveries[duplicateID]?.phase, .notDispatched)
        XCTAssertEqual(committed.deliveries[duplicateID]?.envelope?.clientIntentID, "copy-intent")
        XCTAssertEqual(committed.deliveries[duplicateID]?.envelope?.attachments.count, 1)

        let evidence = GaryxDeliveryEvidencePlanner.plan(
            snapshot: committed,
            correlationID: "original-intent",
            authenticatedScope: scope
        )
        committed = try await fixture.store.commit(try XCTUnwrap(evidence.transaction))
        XCTAssertEqual(committed.deliveries[fixture.deliveryID]?.evidence, .serverAcknowledged)
        XCTAssertEqual(committed.deliveries[fixture.deliveryID]?.phase, .supersededByDuplicate)
        XCTAssertEqual(committed.deliveries[duplicateID]?.evidence, GaryxDeliveryEvidence.none)
    }

    func testCreateResponseLossRestoreSettlesUndispatchedEnvelopeAndConflictAtomically() async throws {
        let fixture = try await makeAmbiguousFixture(deliveryIsAmbiguous: false)
        var snapshot = try await fixture.store.load()
        var create = GaryxCreateDeliveryState(
            scope: scope,
            createIntentID: "original-intent",
            entryID: fixture.entryID
        )
        create.responseLost()
        snapshot = try await fixture.store.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "seed create response loss",
                mutations: [.upsertCreateDelivery(create)]
            )
        )
        let recoveredGeneration = try await fixture.store.allocatePayloadGeneration()
        snapshot = try await fixture.store.load()
        let recoveredID = GaryxComposerPayloadEntryID(rawValue: "create-recovered-entry")
        let conflictID = GaryxPayloadConflictSetID(rawValue: "create-conflict")
        let plan = try XCTUnwrap(
            GaryxCreateDraftRecoveryPlanner.plan(
                snapshot: snapshot,
                key: create.key,
                recoveredEntryID: recoveredID,
                recoveredLifecycleNonce: "create-recovered-token",
                recoveredGeneration: recoveredGeneration,
                conflictSetID: conflictID
            )
        )
        snapshot = try await fixture.store.commit(plan.transaction)

        XCTAssertEqual(snapshot.deliveries[fixture.deliveryID]?.phase, .abandoned)
        XCTAssertEqual(
            snapshot.deliveries[fixture.deliveryID]?.userDisposition,
            .restoredToDraft
        )
        XCTAssertEqual(
            snapshot.createDeliveries[create.key]?.userDisposition,
            .restoredToDraft
        )
        XCTAssertEqual(
            snapshot.payloadStore.entry(recoveredID, scope: scope)?.currentText,
            "sealed message"
        )
        XCTAssertEqual(snapshot.conflicts[conflictID]?.candidates.count, 2)
    }

    func testCreateResponseLossRebuildChangesBothIntentsAndKeepsLateEvidenceIsolated() async throws {
        let fixture = try await makeAmbiguousFixture(deliveryIsAmbiguous: false)
        var snapshot = try await fixture.store.load()
        var create = GaryxCreateDeliveryState(
            scope: scope,
            createIntentID: "original-intent",
            entryID: fixture.entryID
        )
        create.responseLost()
        snapshot = try await fixture.store.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "seed create response loss for rebuild",
                mutations: [.upsertCreateDelivery(create)]
            )
        )
        let duplicateID = GaryxDeliveryRecordID(rawValue: "rebuilt-delivery")
        let plan = try XCTUnwrap(
            GaryxCreateDuplicateRebuildPlanner.plan(
                snapshot: snapshot,
                key: create.key,
                newCreateIntentID: "rebuilt-intent",
                newDeliveryID: duplicateID
            )
        )
        snapshot = try await fixture.store.commit(plan.transaction)

        XCTAssertEqual(
            snapshot.createDeliveries[create.key]?.userDisposition,
            .rebuildMayCreateDuplicateThread
        )
        let newCreateKey = try XCTUnwrap(plan.newCreateKey)
        XCTAssertEqual(snapshot.createDeliveries[newCreateKey]?.phase, .createPending)
        XCTAssertEqual(snapshot.deliveries[fixture.deliveryID]?.phase, .supersededByDuplicate)
        XCTAssertEqual(snapshot.deliveries[duplicateID]?.phase, .notDispatched)
        XCTAssertEqual(
            snapshot.deliveries[duplicateID]?.envelope?.clientIntentID,
            "rebuilt-intent"
        )

        let evidence = GaryxDeliveryEvidencePlanner.plan(
            snapshot: snapshot,
            correlationID: "original-intent",
            authenticatedScope: scope
        )
        snapshot = try await fixture.store.commit(try XCTUnwrap(evidence.transaction))
        XCTAssertEqual(
            snapshot.deliveries[fixture.deliveryID]?.evidence,
            .serverAcknowledged
        )
        XCTAssertEqual(
            snapshot.deliveries[duplicateID]?.evidence,
            GaryxDeliveryEvidence.none
        )
    }

    func testNoticeProjectionRequiresExactEntryInteractionOwner() async throws {
        let fixture = try await makeAmbiguousFixture()
        let snapshot = try await fixture.store.load()
        XCTAssertTrue(
            GaryxComposerDurableNoticeProjector.project(
                snapshot: snapshot,
                hostEntryID: fixture.entryID,
                hasInteractionOwner: false
            ).isEmpty
        )
        let notices = GaryxComposerDurableNoticeProjector.project(
            snapshot: snapshot,
            hostEntryID: fixture.entryID,
            hasInteractionOwner: true
        )
        XCTAssertEqual(notices.first?.title, "Send status unknown")
        XCTAssertEqual(
            notices.first?.actions,
            [.restoreDelivery(fixture.deliveryID), .resendDeliveryCopy(fixture.deliveryID)]
        )
    }

    func testBackpressureFeedbackPersistsAndAcknowledgesOnlyInOwnerTransaction() async throws {
        let fixture = try await makeAmbiguousFixture()
        var snapshot = try await fixture.store.load()
        for index in 1..<GaryxDeliveryQuota.perScopeRecordLimit {
            let reservation = GaryxSendReservationID(rawValue: UInt64(100 + index))
            var ledger = GaryxProvisionalReservationLedger(
                key: .init(scope: scope, entryID: fixture.entryID, reservationID: reservation),
                envelopeGeneration: 10,
                followupGeneration: 11
            )
            XCTAssertTrue(ledger.settle(.committed, targetGeneration: 11))
            let record = GaryxDeliveryRecord(
                id: .init(rawValue: "quota-delivery-\(index)"),
                scope: scope,
                entryID: fixture.entryID,
                reservationID: reservation,
                correlationID: "quota-intent-\(index)",
                envelope: .init(
                    text: "pending",
                    attachmentIDs: [],
                    generation: 10,
                    clientIntentID: "quota-intent-\(index)"
                )
            )
            snapshot = try await fixture.store.commit(
                .init(
                    expectedRevision: snapshot.revision,
                    label: "fill durable delivery quota",
                    mutations: [.upsertLedger(ledger), .upsertDelivery(record)]
                )
            )
        }
        let feedbackID = GaryxFeedbackID(rawValue: "backpressure-feedback")
        let feedbackPlan = try XCTUnwrap(
            GaryxDeliveryBackpressurePlanner.plan(
                snapshot: snapshot,
                entryID: fixture.entryID,
                envelopeBytes: 1,
                feedbackID: feedbackID
            )
        )
        snapshot = try await fixture.store.commit(feedbackPlan)
        XCTAssertEqual(snapshot.feedback[feedbackID]?.phase, .pending)
        XCTAssertTrue(
            snapshot.payloadStore.entry(fixture.entryID, scope: scope)?
                .feedbackReferences.contains(feedbackID) == true
        )

        let presentation = try XCTUnwrap(
            GaryxFeedbackPresentationPlanner.plan(
                snapshot: snapshot,
                hostEntryID: fixture.entryID,
                hasInteractionOwner: true
            )
        )
        snapshot = try await fixture.store.commit(presentation)
        XCTAssertEqual(snapshot.feedback[feedbackID]?.phase, .presented)
        let acknowledgement = try XCTUnwrap(
            GaryxFeedbackAcknowledgementPlanner.plan(
                snapshot: snapshot,
                feedbackID: feedbackID,
                hostEntryID: fixture.entryID
            )
        )
        snapshot = try await fixture.store.commit(acknowledgement)
        XCTAssertEqual(snapshot.feedback[feedbackID]?.phase, .acknowledged)
        XCTAssertFalse(
            snapshot.payloadStore.entry(fixture.entryID, scope: scope)?
                .feedbackReferences.contains(feedbackID) == true
        )
    }

    func testScopeRevokePersistsWatermarkAndSettlesAmbiguousEvidenceWithoutDomainIngress() async throws {
        let fixture = try await makeAmbiguousFixture()
        let snapshot = try await fixture.store.load()
        let plan = try XCTUnwrap(
            GaryxGatewayScopeSettlementPlanner.revoke(snapshot: snapshot, scope: scope)
        )
        let revoked = try await fixture.store.commit(plan.transaction)
        XCTAssertEqual(revoked.scopeRegistry.revokedThroughEpoch[scope.identity], scope.epoch)
        XCTAssertEqual(revoked.scopeRegistry.lifecycle(of: scope), .revoked)
        XCTAssertEqual(revoked.deliveries[fixture.deliveryID]?.phase, .evidence)
        XCTAssertEqual(
            revoked.deliveries[fixture.deliveryID]?.userDisposition,
            .scopeRevoked
        )

        let lateEvidence = GaryxDeliveryEvidencePlanner.plan(
            snapshot: revoked,
            correlationID: "original-intent",
            authenticatedScope: scope
        )
        let claimed = try await fixture.store.commit(try XCTUnwrap(lateEvidence.transaction))
        XCTAssertEqual(claimed.deliveries[fixture.deliveryID]?.evidence, .serverAcknowledged)
        XCTAssertEqual(claimed.scopeRegistry.lifecycle(of: scope), .revoked)
    }

    private func makeAmbiguousFixture(
        deliveryIsAmbiguous: Bool = true
    ) async throws -> (
        store: GaryxFakeComposerDurabilityStore,
        entryID: GaryxComposerPayloadEntryID,
        deliveryID: GaryxDeliveryRecordID
    ) {
        let entryID = GaryxComposerPayloadEntryID(rawValue: "delivery-entry")
        let deliveryID = GaryxDeliveryRecordID(rawValue: "ambiguous-delivery")
        let reservationID = GaryxSendReservationID(rawValue: 9)
        var registry = GaryxGatewayScopeRegistry(initialActiveScope: scope)
        _ = registry.switchActive(to: scope)
        let store = GaryxFakeComposerDurabilityStore(
            initial: .init(
                scopeRegistry: registry,
                generationHighWatermark: 32,
                reservationHighWatermark: 32
            )
        )
        var entry = GaryxComposerPayloadEntry(
            id: entryID,
            scope: scope,
            destination: .thread("thread::test"),
            lifecycleToken: .init(entryID: entryID, nonce: "delivery-token"),
            currentGeneration: 11,
            text: "live follow-up"
        )
        entry.addDeliveryReference(deliveryID)
        var ledger = GaryxProvisionalReservationLedger(
            key: .init(scope: scope, entryID: entryID, reservationID: reservationID),
            envelopeGeneration: 10,
            followupGeneration: 11
        )
        XCTAssertTrue(ledger.settle(.committed, targetGeneration: 11))
        let attachment = GaryxComposerAttachment(
            id: .init(rawValue: "attachment-1"),
            stagedAssetID: .init(rawValue: "already-uploaded-1"),
            generation: 10,
            byteCount: 12,
            kind: "image",
            name: "file.png",
            mediaType: "image/png",
            uploadedPath: "prompt/file.png"
        )
        var delivery = GaryxDeliveryRecord(
            id: deliveryID,
            scope: scope,
            entryID: entryID,
            reservationID: reservationID,
            correlationID: "original-intent",
            envelope: .init(
                text: "sealed message",
                attachmentIDs: [attachment.id],
                attachments: [attachment],
                generation: 10,
                clientIntentID: "original-intent"
            )
        )
        if deliveryIsAmbiguous {
            XCTAssertTrue(delivery.markTransportAttempted())
            XCTAssertTrue(delivery.markAmbiguous())
        }
        _ = try await store.commit(
            .init(
                expectedRevision: 0,
                label: "seed ambiguous delivery fixture",
                mutations: [.upsertLedger(ledger), .upsertEntry(entry), .upsertDelivery(delivery)]
            )
        )
        return (store, entryID, deliveryID)
    }
}
