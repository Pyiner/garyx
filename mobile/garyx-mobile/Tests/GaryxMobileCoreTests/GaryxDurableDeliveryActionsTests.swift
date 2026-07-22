import XCTest
@testable import GaryxMobileCore

final class GaryxDurableDeliveryActionsTests: XCTestCase {
    private let scope = GaryxGatewayScope(identity: "test-gateway", epoch: 4)

    func testUndispatchedRecoveryAdoptsIntoWhitespaceOnlyHostWithoutNotice() async throws {
        let fixture = try await makeAmbiguousFixture(
            deliveryIsAmbiguous: false,
            currentText: " \n\t "
        )
        var snapshot = try await fixture.store.load()
        let generation = try await fixture.store.allocatePayloadGeneration()
        snapshot = try await fixture.store.load()
        let recoveredID = GaryxComposerPayloadEntryID(rawValue: "unused-recovered-entry")
        let plan = try XCTUnwrap(
            GaryxUndispatchedDeliveryRecoveryPlanner.plan(
                snapshot: snapshot,
                deliveryID: fixture.deliveryID,
                recoveredEntryID: recoveredID,
                recoveredLifecycleNonce: "unused-recovered-token",
                recoveredGeneration: generation,
                conflictSetID: .init(rawValue: "unused-conflict"),
                incompleteAttachmentFeedbackID: .init(rawValue: "unused-feedback")
            )
        )
        XCTAssertEqual(plan.placement, .adoptedIntoHost)
        snapshot = try await fixture.store.commit(plan.transaction)

        let host = try XCTUnwrap(snapshot.payloadStore.entry(fixture.entryID, scope: scope))
        XCTAssertEqual(host.currentText, "sealed message")
        XCTAssertEqual(host.attachments.values.first?.uploadedPath, "prompt/file.png")
        XCTAssertNil(snapshot.payloadStore.entry(recoveredID, scope: scope))
        XCTAssertTrue(snapshot.conflicts.isEmpty)
        XCTAssertTrue(
            GaryxComposerDurableNoticeProjector.project(
                snapshot: snapshot,
                hostEntryID: fixture.entryID,
                hasInteractionOwner: true
            ).isEmpty
        )
    }

    func testUndispatchedRecoveryDefersBehindCurrentDraftAndReclaimsQuota() async throws {
        let fixture = try await makeAmbiguousFixture(deliveryIsAmbiguous: false)
        var snapshot = try await fixture.store.load()
        let generation = try await fixture.store.allocatePayloadGeneration()
        snapshot = try await fixture.store.load()
        let recoveredID = GaryxComposerPayloadEntryID(rawValue: "undispatched-recovered")
        let conflictID = GaryxPayloadConflictSetID(rawValue: "undispatched-conflict")
        let plan = try XCTUnwrap(
            GaryxUndispatchedDeliveryRecoveryPlanner.plan(
                snapshot: snapshot,
                deliveryID: fixture.deliveryID,
                recoveredEntryID: recoveredID,
                recoveredLifecycleNonce: "undispatched-token",
                recoveredGeneration: generation,
                conflictSetID: conflictID,
                incompleteAttachmentFeedbackID: .init(rawValue: "undispatched-feedback")
            )
        )
        XCTAssertEqual(
            plan.placement,
            .deferred(entryID: recoveredID, conflictSetID: conflictID)
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
        XCTAssertEqual(
            snapshot.payloadStore.entry(recoveredID, scope: scope)?.attachments.values
                .first?.uploadedPath,
            "prompt/file.png"
        )
        XCTAssertEqual(snapshot.conflicts[conflictID]?.candidates.count, 2)
        let quota = GaryxDeliveryQuota(rebuilding: Array(snapshot.deliveries.values))
        XCTAssertEqual(quota.nonTerminalGlobal, 0)
        XCTAssertEqual(quota.nonTerminalByScope[scope] ?? 0, 0)
        XCTAssertEqual(
            GaryxComposerDurableNoticeProjector.project(
                snapshot: snapshot,
                hostEntryID: fixture.entryID,
                hasInteractionOwner: true
            ).map(\.kind),
            []
        )
    }

    func testLegacyUndispatchedAttachmentRecoveryKeepsTextAndWarnsDurably() async throws {
        let fixture = try await makeAmbiguousFixture(
            deliveryIsAmbiguous: false,
            includeAttachmentSnapshot: false
        )
        var snapshot = try await fixture.store.load()
        let generation = try await fixture.store.allocatePayloadGeneration()
        snapshot = try await fixture.store.load()
        let recoveredID = GaryxComposerPayloadEntryID(rawValue: "legacy-recovered")
        let feedbackID = GaryxFeedbackID(rawValue: "legacy-recovery-feedback")
        let plan = try XCTUnwrap(
            GaryxUndispatchedDeliveryRecoveryPlanner.plan(
                snapshot: snapshot,
                deliveryID: fixture.deliveryID,
                recoveredEntryID: recoveredID,
                recoveredLifecycleNonce: "legacy-recovered-token",
                recoveredGeneration: generation,
                conflictSetID: .init(rawValue: "legacy-conflict"),
                incompleteAttachmentFeedbackID: feedbackID
            )
        )
        XCTAssertEqual(plan.unrestoredAttachmentIDs, [.init(rawValue: "attachment-1")])
        snapshot = try await fixture.store.commit(plan.transaction)

        XCTAssertEqual(snapshot.deliveries[fixture.deliveryID]?.phase, .abandoned)
        XCTAssertEqual(
            snapshot.payloadStore.entry(recoveredID, scope: scope)?.currentText,
            "sealed message"
        )
        XCTAssertTrue(
            snapshot.payloadStore.entry(recoveredID, scope: scope)?.attachments.isEmpty == true
        )
        XCTAssertEqual(
            snapshot.feedback[feedbackID]?.kind,
            .deliveryAttachmentRecoveryIncomplete
        )
        XCTAssertTrue(
            snapshot.payloadStore.entry(fixture.entryID, scope: scope)?
                .feedbackReferences.contains(feedbackID) == true
        )
        let notices = GaryxComposerDurableNoticeProjector.project(
            snapshot: snapshot,
            hostEntryID: fixture.entryID,
            hasInteractionOwner: true
        )
        XCTAssertEqual(notices.map(\.kind), [.feedback])
        XCTAssertEqual(notices.first?.title, "Some attachments could not be restored")

        let acknowledgement = try XCTUnwrap(
            GaryxFeedbackAcknowledgementPlanner.plan(
                snapshot: snapshot,
                feedbackID: feedbackID,
                hostEntryID: fixture.entryID
            )
        )
        snapshot = try await fixture.store.commit(acknowledgement)
        XCTAssertEqual(snapshot.feedback[feedbackID]?.phase, .acknowledged)
    }

    func testRestoreExitDefersWithoutOverwritingThenAdoptsWhenHostBecomesEmpty() async throws {
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

        var emptiedHost = try XCTUnwrap(
            snapshot.payloadStore.entry(fixture.entryID, scope: scope)
        )
        emptiedHost.setText("", generation: emptiedHost.currentGeneration)
        snapshot = try await fixture.store.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "finish current draft before deferred adoption",
                mutations: [.upsertEntry(emptiedHost)]
            )
        )
        let candidate = try XCTUnwrap(
            GaryxDeferredDraftAdoptionPlanner.candidate(
                snapshot: snapshot,
                hostEntryID: fixture.entryID
            )
        )
        let replacementGeneration = try await fixture.store.allocatePayloadGeneration()
        snapshot = try await fixture.store.load()
        let adoption = try XCTUnwrap(
            GaryxDeferredDraftAdoptionPlanner.plan(
                snapshot: snapshot,
                candidate: candidate,
                replacementGeneration: replacementGeneration
            )
        )
        snapshot = try await fixture.store.commit(adoption.transaction)
        let host = try XCTUnwrap(snapshot.payloadStore.entry(fixture.entryID, scope: scope))
        XCTAssertEqual(host.currentText, "sealed message")
        XCTAssertEqual(host.attachments.values.first?.uploadedPath, "prompt/file.png")
        XCTAssertNil(snapshot.payloadStore.entry(recoveredID, scope: scope))
        XCTAssertNil(snapshot.conflicts[conflictID])
    }

    func testDeferredAdoptionUsesRecoveryGenerationOrder() throws {
        let hostID = GaryxComposerPayloadEntryID(rawValue: "ordered-host")
        let earlyID = GaryxComposerPayloadEntryID(rawValue: "ordered-early")
        let lateID = GaryxComposerPayloadEntryID(rawValue: "ordered-late")
        var store = GaryxComposerPayloadStore()
        for (id, generation, text) in [
            (hostID, UInt64(10), ""),
            (earlyID, UInt64(12), "early recovery"),
            (lateID, UInt64(13), "late recovery"),
        ] {
            XCTAssertTrue(
                store.insert(
                    .init(
                        id: id,
                        scope: scope,
                        destination: .draft(id.rawValue),
                        lifecycleToken: .init(entryID: id, nonce: "token-\(id.rawValue)"),
                        currentGeneration: generation,
                        text: text
                    )
                )
            )
        }

        func conflict(
            id: GaryxPayloadConflictSetID,
            recoveredEntryID: GaryxComposerPayloadEntryID
        ) -> GaryxPayloadConflictSet {
            var value = GaryxPayloadConflictSet(id: id, scope: scope)
            XCTAssertTrue(
                value.admitCandidate(
                    .init(entryID: hostID, label: "Current draft"),
                    membershipDurabilityAvailable: true
                )
            )
            XCTAssertTrue(
                value.admitCandidate(
                    .init(entryID: recoveredEntryID, label: "Recovered send"),
                    membershipDurabilityAvailable: true
                )
            )
            return value
        }

        let lateConflict = conflict(
            id: .init(rawValue: "a-lexically-first-late"),
            recoveredEntryID: lateID
        )
        let earlyConflict = conflict(
            id: .init(rawValue: "z-lexically-last-early"),
            recoveredEntryID: earlyID
        )
        var queuedStore = store
        let newDeliveryID = GaryxDeliveryRecordID(rawValue: "ordered-new-recovery")
        var queuedHost = try XCTUnwrap(queuedStore.entry(hostID, scope: scope))
        queuedHost.addDeliveryReference(newDeliveryID)
        queuedStore.update(queuedHost)
        let newDelivery = GaryxDeliveryRecord(
            id: newDeliveryID,
            scope: scope,
            entryID: hostID,
            reservationID: .init(rawValue: 14),
            correlationID: "ordered-new-intent",
            envelope: .init(
                text: "new recovery",
                attachmentIDs: [],
                generation: 14,
                clientIntentID: "ordered-new-intent"
            )
        )
        let snapshot = GaryxComposerDurabilitySnapshot(
            payloadStore: queuedStore,
            conflicts: [
                lateConflict.id: lateConflict,
                earlyConflict.id: earlyConflict,
            ],
            deliveries: [newDeliveryID: newDelivery],
            generationHighWatermark: 32
        )

        XCTAssertEqual(
            GaryxDeferredDraftAdoptionPlanner.candidate(
                snapshot: snapshot,
                hostEntryID: hostID
            )?.recoveredEntryID,
            earlyID
        )
        let newRecovery = try XCTUnwrap(
            GaryxUndispatchedDeliveryRecoveryPlanner.plan(
                snapshot: snapshot,
                deliveryID: newDeliveryID,
                recoveredEntryID: .init(rawValue: "ordered-new-entry"),
                recoveredLifecycleNonce: "ordered-new-token",
                recoveredGeneration: 15,
                conflictSetID: .init(rawValue: "ordered-new-conflict"),
                incompleteAttachmentFeedbackID: .init(rawValue: "ordered-new-feedback")
            )
        )
        XCTAssertEqual(
            newRecovery.placement,
            .deferred(
                entryID: .init(rawValue: "ordered-new-entry"),
                conflictSetID: .init(rawValue: "ordered-new-conflict")
            ),
            "a newer recovery must not jump ahead when an older deferred payload is ready"
        )
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

    func testCreateResponseLossRestoreDefersBehindCurrentDraftAtomically() async throws {
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
        XCTAssertEqual(
            plan.placement,
            .deferred(entryID: recoveredID, conflictSetID: conflictID)
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
        XCTAssertTrue(
            GaryxComposerDurableNoticeProjector.project(
                snapshot: snapshot,
                hostEntryID: fixture.entryID,
                hasInteractionOwner: true
            ).isEmpty
        )
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

    func testComposerProjectsNoInlineStatusAcrossNetworkFailureRecoverySequence() async throws {
        let fixture = try await makeAmbiguousFixture(deliveryIsAmbiguous: false)

        func assertComposerHasNoInlineStatus(
            _ snapshot: GaryxComposerDurabilitySnapshot,
            stage: String,
            file: StaticString = #filePath,
            line: UInt = #line
        ) {
            let titles = GaryxComposerDurableNoticeProjector.project(
                snapshot: snapshot,
                hostEntryID: fixture.entryID,
                hasInteractionOwner: true
            ).map(\.title)
            XCTAssertEqual(
                titles,
                [],
                "composer emitted inline status during \(stage): \(titles)",
                file: file,
                line: line
            )
        }

        var snapshot = try await fixture.store.load()
        assertComposerHasNoInlineStatus(snapshot, stage: "outbox committed")

        var delivery = try XCTUnwrap(snapshot.deliveries[fixture.deliveryID])
        XCTAssertTrue(delivery.markTransportAttempted())
        snapshot = try await fixture.store.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "simulate a stuck transport attempt",
                mutations: [.upsertDelivery(delivery)]
            )
        )
        assertComposerHasNoInlineStatus(snapshot, stage: "transport stuck")

        XCTAssertTrue(delivery.markAmbiguous())
        snapshot = try await fixture.store.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "simulate response loss during network jitter",
                mutations: [.upsertDelivery(delivery)]
            )
        )
        assertComposerHasNoInlineStatus(snapshot, stage: "network response lost")

        let recoveredGeneration = try await fixture.store.allocatePayloadGeneration()
        snapshot = try await fixture.store.load()
        let recovery = try XCTUnwrap(
            GaryxDeliveryDraftRecoveryPlanner.plan(
                snapshot: snapshot,
                deliveryID: fixture.deliveryID,
                recoveredEntryID: .init(rawValue: "network-recovery-entry"),
                recoveredLifecycleNonce: "network-recovery-token",
                recoveredGeneration: recoveredGeneration,
                conflictSetID: .init(rawValue: "network-recovery-conflict")
            )
        )
        snapshot = try await fixture.store.commit(recovery.transaction)
        assertComposerHasNoInlineStatus(snapshot, stage: "durable recovery completed")
    }

    private func makeAmbiguousFixture(
        deliveryIsAmbiguous: Bool = true,
        includeAttachmentSnapshot: Bool = true,
        currentText: String = "live follow-up"
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
            text: currentText
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
                attachments: includeAttachmentSnapshot ? [attachment] : [],
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
