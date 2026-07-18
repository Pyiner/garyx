import XCTest
@testable import GaryxMobileCore

final class GaryxComposerDurabilityStoreTests: XCTestCase {
    private let scope = GaryxGatewayScope(identity: "gateway", epoch: 1)

    func testProtocolExposesSingleTransactionalDurabilitySeam() async throws {
        let fake = GaryxFakeComposerDurabilityStore()
        let store: any GaryxComposerDurabilityStore = fake
        let initial = try await store.load()
        XCTAssertEqual(initial.revision, 0)

        let entry = makeEntry()
        let committed = try await store.commit(
            GaryxComposerDurabilityTransaction(
                expectedRevision: 0,
                label: "insert entry",
                mutations: [.upsertEntry(entry)]
            )
        )
        XCTAssertEqual(committed.revision, 1)
        XCTAssertEqual(committed.payloadStore.entry(entry.id, scope: scope), entry)
    }

    func testRevisionCASLetsOnlyOneConcurrentWriterWin() async throws {
        let fake = GaryxFakeComposerDurabilityStore()
        let first = makeEntry(id: "first")
        let second = makeEntry(id: "second")

        async let firstResult = commitResult(
            store: fake,
            transaction: .init(
                expectedRevision: 0,
                label: "first",
                mutations: [.upsertEntry(first)]
            )
        )
        async let secondResult = commitResult(
            store: fake,
            transaction: .init(
                expectedRevision: 0,
                label: "second",
                mutations: [.upsertEntry(second)]
            )
        )
        let results = await [firstResult, secondResult]
        XCTAssertEqual(results.filter(\.isSuccess).count, 1)
        XCTAssertEqual(results.filter { !$0.isSuccess }.count, 1)
        let snapshot = try await fake.load()
        XCTAssertEqual(snapshot.revision, 1)
        let present = [first, second].filter {
            snapshot.payloadStore.entry($0.id, scope: scope) != nil
        }
        XCTAssertEqual(present.count, 1)
    }

    func testInjectedFailureAtEveryMutationBoundaryPublishesNothing() async throws {
        let entry = makeEntry()
        let feedback = makeFeedback()
        let operation = makeOperation(state: .failedTerminal)
        let mutations: [GaryxComposerDurabilityMutation] = [
            .upsertEntry(entry),
            .upsertFeedback(feedback),
            .upsertOperation(operation),
        ]

        for index in mutations.indices {
            let fake = GaryxFakeComposerDurabilityStore()
            await fake.injectFailure(atMutationIndex: index)
            do {
                _ = try await fake.commit(
                    .init(expectedRevision: 0, label: "failpoint", mutations: mutations)
                )
                XCTFail("expected injected failure at \(index)")
            } catch {
                XCTAssertEqual(
                    error as? GaryxComposerDurabilityError,
                    .injectedFailure(mutationIndex: index)
                )
            }
            let snapshot = try await fake.load()
            XCTAssertEqual(snapshot, GaryxComposerDurabilitySnapshot())
        }
    }

    func testLedgerMustPrecedeEveryReservationDescendant() async throws {
        let ledger = makeLedger(outcome: nil)
        let manifest = makeManifest(reservationID: reservation)
        let operation = GaryxOperationCapability(
            context: GaryxScopeBoundOperationContext(
                key: manifest.key,
                clientIdentity: "client",
                configurationFingerprint: "configuration",
                payloadLifecycle: GaryxPayloadLifecycleCapture(
                    token: makeEntry().lifecycle.token,
                    revision: makeEntry().lifecycle.revision
                )
            ),
            state: .preparing
        )
        let replacement = makeReplacement(reservationID: reservation)
        let drained = makeDurableProducerDrained(reservationID: reservation)

        for descendant in [
            GaryxComposerDurabilityMutation.upsertOperation(operation),
            GaryxComposerDurabilityMutation.upsertManifest(manifest),
            .upsertReplacement(replacement),
            .upsertProducerDrained(drained.key, drained.value),
        ] {
            let fake = GaryxFakeComposerDurabilityStore()
            do {
                _ = try await fake.commit(
                    .init(
                        expectedRevision: 0,
                        label: "wrong order",
                        mutations: [descendant, .upsertLedger(ledger)]
                    )
                )
                XCTFail("descendant committed before ledger")
            } catch let error as GaryxComposerDurabilityError {
                guard case .invariantViolation = error else {
                    XCTFail("unexpected error \(error)")
                    continue
                }
            }
            let unchanged = try await fake.load()
            XCTAssertEqual(unchanged, GaryxComposerDurabilitySnapshot())
        }

        let fake = GaryxFakeComposerDurabilityStore()
        var entry = makeEntry()
        entry.addOperation(operation.context.key)
        let committed = try await fake.commit(
            .init(
                expectedRevision: 0,
                label: "correct admission order",
                mutations: [
                    .upsertLedger(ledger),
                    .upsertEntry(entry),
                    .upsertOperation(operation),
                    .upsertManifest(manifest),
                    .upsertReplacement(replacement),
                    .upsertProducerDrained(drained.key, drained.value),
                ]
            )
        )
        XCTAssertEqual(committed.ledgers.count, 1)
        XCTAssertEqual(committed.operations.count, 1)
        XCTAssertEqual(committed.manifests.count, 1)
        XCTAssertEqual(committed.replacements.count, 1)
        XCTAssertEqual(committed.producerDrained.count, 1)
    }

    func testProducerDrainedKeyMustMatchItsPayloadIdentity() async throws {
        let ledger = makeLedger(outcome: nil)
        let drained = makeDurableProducerDrained(reservationID: reservation)
        let mismatched = GaryxDurableProducerDrainedRecord(
            scope: drained.value.scope,
            entryID: drained.value.entryID,
            reservationID: drained.value.reservationID,
            record: .init(
                sessionID: .init(rawValue: "different-session"),
                epoch: drained.key.epoch,
                finalSequence: drained.value.record.finalSequence,
                bufferedText: drained.value.record.bufferedText
            )
        )
        let fake = GaryxFakeComposerDurabilityStore()

        do {
            _ = try await fake.commit(
                .init(
                    expectedRevision: 0,
                    label: "reject mismatched producerDrained identity",
                    mutations: [
                        .upsertLedger(ledger),
                        .upsertProducerDrained(drained.key, mismatched),
                    ]
                )
            )
            XCTFail("producerDrained key must match its payload identity")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected error \(error)")
            }
        }
        let unchanged = try await fake.load()
        XCTAssertEqual(unchanged, GaryxComposerDurabilitySnapshot())
    }

    func testSyntheticReservationRecoveryExecutesAllFiveStepsAtomically() async throws {
        var entry = makeEntry(text: "T")
        entry.setText("U", generation: 11)
        let operationKey = GaryxOperationCapabilityKey(
            scope: scope,
            entryID: entry.id,
            generation: 11,
            reservationID: reservation,
            branch: .followup,
            operationID: GaryxOperationID(rawValue: "recovery-operation")
        )
        entry.addOperation(operationKey)
        let assetID = GaryxStagedAssetID(rawValue: "recovery-asset")
        let operation = GaryxOperationCapability(
            context: GaryxScopeBoundOperationContext(
                key: operationKey,
                clientIdentity: "client",
                configurationFingerprint: "configuration",
                payloadLifecycle: GaryxPayloadLifecycleCapture(
                    token: entry.lifecycle.token,
                    revision: entry.lifecycle.revision
                )
            ),
            state: .preparing,
            stagedAssetID: assetID,
            reservedBytes: 32
        )
        let manifest = GaryxOperationManifest(
            key: operationKey,
            stagedPath: "staging/recovery.bin",
            state: .preparing,
            uploadAttempted: false
        )
        let replacement = GaryxReplacementRecord(
            id: GaryxReplacementID(rawValue: "recovery-replacement"),
            scope: scope,
            entryID: entry.id,
            oldKey: operationKey,
            reservationID: reservation,
            branch: .followup,
            stagedAssetID: assetID,
            reservedBytes: 32
        )
        var payloadStore = GaryxComposerPayloadStore()
        XCTAssertTrue(payloadStore.insert(entry))
        XCTAssertTrue(payloadStore.insert(makeEntry(id: "collision", text: "other")))
        let ledger = makeLedger(outcome: nil)
        var barrier = makeBarrier()
        XCTAssertEqual(
            barrier.seal(
                reservationID: reservation,
                envelope: makeEnvelope(text: "T"),
                followupGeneration: 11,
                readiness: .ready,
                quota: .init(),
                producerPhase: .live,
                lifecycle: entry.lifecycle.snapshot
            ),
            .sealed
        )
        XCTAssertTrue(barrier.replaceProvisionalText("U", lifecycle: entry.lifecycle.snapshot))
        let drained = makeDurableProducerDrained(reservationID: reservation)
        let initial = GaryxComposerDurabilitySnapshot(
            payloadStore: payloadStore,
            operations: [operationKey: operation],
            manifests: [operationKey: manifest],
            replacements: [replacement.id: replacement],
            barriers: [entry.id: barrier],
            ledgers: [ledger.key: ledger],
            producerDrained: [drained.key: drained.value],
            stagedAssetOwners: [assetID: operationKey],
            stagedAssetReservedBytes: [assetID: 32],
            reservedBytes: 32,
            generationHighWatermark: 32,
            reservationHighWatermark: 32
        )
        let conflictID = GaryxPayloadConflictSetID(rawValue: "synthetic-recovery-conflict")
        let plan = try XCTUnwrap(
            GaryxSyntheticReservationRecoveryPlanner.plan(
                snapshot: initial,
                ledgerKey: ledger.key,
                mergeGeneration: 12,
                conflictSetID: conflictID
            )
        )
        XCTAssertEqual(plan.performedSteps, GaryxSyntheticRecoveryStep.allCases)
        XCTAssertGreaterThanOrEqual(plan.transaction.mutations.count, 5)

        for failpoint in plan.transaction.mutations.indices {
            let isolated = GaryxFakeComposerDurabilityStore(initial: initial)
            await isolated.injectFailure(atMutationIndex: failpoint)
            do {
                _ = try await isolated.commit(plan.transaction)
                XCTFail("expected synthetic recovery failpoint \(failpoint)")
            } catch {
                XCTAssertEqual(
                    error as? GaryxComposerDurabilityError,
                    .injectedFailure(mutationIndex: failpoint)
                )
            }
            let unchanged = try await isolated.load()
            XCTAssertEqual(unchanged, initial)
        }

        let fake = GaryxFakeComposerDurabilityStore(initial: initial)
        let recovered = try await fake.commit(plan.transaction)
        XCTAssertEqual(recovered.ledgers[ledger.key]?.terminalOutcome, .revoked)
        XCTAssertEqual(recovered.ledgers[ledger.key]?.targetMapping?.generation, 12)
        XCTAssertEqual(recovered.claimedGenerations, [12])
        XCTAssertEqual(recovered.payloadStore.entry(entry.id, scope: scope)?.currentGeneration, 12)
        XCTAssertEqual(recovered.payloadStore.entry(entry.id, scope: scope)?.currentText, "TU")
        XCTAssertEqual(recovered.barriers[entry.id]?.phase, .revoked)
        XCTAssertNil(recovered.producerDrained[drained.key])
        XCTAssertEqual(recovered.recoveredInputClosures[drained.key]?.finalText, "TU")
        XCTAssertEqual(recovered.recoveredInputClosures[drained.key]?.targetGeneration, 12)
        XCTAssertEqual(recovered.recoveredInputClosures[drained.key]?.closePublicationCount, 1)
        let remappedKey = operationKey.remapped(toGeneration: 12)
        XCTAssertNil(recovered.operations[operationKey])
        XCTAssertEqual(recovered.operations[remappedKey]?.state, .preparing)
        XCTAssertNil(recovered.manifests[operationKey])
        XCTAssertEqual(recovered.manifests[remappedKey]?.key, remappedKey)
        XCTAssertEqual(recovered.stagedAssetOwners[assetID], remappedKey)
        XCTAssertEqual(recovered.replacements[replacement.id]?.oldKey, remappedKey)
        XCTAssertEqual(
            Set(recovered.conflicts[conflictID]?.candidates.map(\.entryID) ?? []),
            [entry.id, GaryxComposerPayloadEntryID(rawValue: "collision")]
        )
        XCTAssertNil(
            GaryxSyntheticReservationRecoveryPlanner.plan(
                snapshot: recovered,
                ledgerKey: ledger.key,
                mergeGeneration: 13,
                conflictSetID: conflictID
            ),
            "terminal recovery is exactly once"
        )
    }

    func testSyntheticRecoveryAfterProducerDrainedWithoutOperationsClosesMergedInput() async throws {
        var entry = makeEntry(text: "T")
        entry.setText("U", generation: 11)
        var payloadStore = GaryxComposerPayloadStore()
        XCTAssertTrue(payloadStore.insert(entry))
        let ledger = makeLedger(outcome: nil)
        var barrier = makeBarrier()
        XCTAssertEqual(
            barrier.seal(
                reservationID: reservation,
                envelope: makeEnvelope(text: "T"),
                followupGeneration: 11,
                readiness: .ready,
                quota: .init(),
                producerPhase: .live,
                lifecycle: entry.lifecycle.snapshot
            ),
            .sealed
        )
        XCTAssertTrue(barrier.replaceProvisionalText("U", lifecycle: entry.lifecycle.snapshot))
        let drained = makeDurableProducerDrained(reservationID: reservation)
        let persisted = GaryxComposerDurabilitySnapshot(
            payloadStore: payloadStore,
            barriers: [entry.id: barrier],
            ledgers: [ledger.key: ledger],
            producerDrained: [drained.key: drained.value],
            generationHighWatermark: 32,
            reservationHighWatermark: 32
        )
        let relaunched = try JSONDecoder().decode(
            GaryxComposerDurabilitySnapshot.self,
            from: JSONEncoder().encode(persisted)
        )
        let plan = try XCTUnwrap(
            GaryxSyntheticReservationRecoveryPlanner.plan(
                snapshot: relaunched,
                ledgerKey: ledger.key,
                mergeGeneration: 12
            )
        )

        let fake = GaryxFakeComposerDurabilityStore(initial: relaunched)
        let recovered = try await fake.commit(plan.transaction)
        XCTAssertTrue(recovered.operations.isEmpty)
        XCTAssertTrue(recovered.manifests.isEmpty)
        XCTAssertEqual(recovered.payloadStore.entry(entry.id, scope: scope)?.currentText, "TU")
        XCTAssertEqual(recovered.payloadStore.entry(entry.id, scope: scope)?.currentGeneration, 12)
        XCTAssertEqual(recovered.recoveredInputClosures[drained.key]?.finalText, "TU")
        XCTAssertEqual(recovered.recoveredInputClosures[drained.key]?.closePublicationCount, 1)
        XCTAssertNil(recovered.producerDrained[drained.key])
    }

    func testSyntheticRecoveryAfterRelaunchKeepsFollowupAttachmentOutOfSealedEnvelope() async throws {
        var entry = makeEntry(text: "T")
        entry.setText("U", generation: 11)
        let envelopeAttachment = GaryxComposerAttachment(
            id: GaryxAttachmentID(rawValue: "envelope-attachment"),
            stagedAssetID: GaryxStagedAssetID(rawValue: "envelope-asset"),
            generation: 10,
            byteCount: 10
        )
        let followupAttachment = GaryxComposerAttachment(
            id: GaryxAttachmentID(rawValue: "followup-attachment"),
            stagedAssetID: GaryxStagedAssetID(rawValue: "followup-asset"),
            generation: 11,
            byteCount: 11
        )
        entry.addAttachment(envelopeAttachment)
        entry.addAttachment(followupAttachment)
        var payloadStore = GaryxComposerPayloadStore()
        XCTAssertTrue(payloadStore.insert(entry))
        let ledger = makeLedger(outcome: nil)
        var barrier = makeBarrier()
        let envelope = GaryxDeliveryEnvelope(
            text: "T",
            attachmentIDs: [envelopeAttachment.id],
            generation: 10,
            clientIntentID: "intent"
        )
        XCTAssertEqual(
            barrier.seal(
                reservationID: reservation,
                envelope: envelope,
                followupGeneration: 11,
                readiness: .ready,
                quota: .init(),
                producerPhase: .live,
                lifecycle: entry.lifecycle.snapshot
            ),
            .sealed
        )
        XCTAssertTrue(barrier.replaceProvisionalText("U", lifecycle: entry.lifecycle.snapshot))
        XCTAssertTrue(
            barrier.addProvisionalAttachment(
                followupAttachment.id,
                lifecycle: entry.lifecycle.snapshot
            )
        )
        let drained = makeDurableProducerDrained(reservationID: reservation)
        let persisted = GaryxComposerDurabilitySnapshot(
            payloadStore: payloadStore,
            barriers: [entry.id: barrier],
            ledgers: [ledger.key: ledger],
            producerDrained: [drained.key: drained.value],
            generationHighWatermark: 32,
            reservationHighWatermark: 32
        )
        let relaunched = try JSONDecoder().decode(
            GaryxComposerDurabilitySnapshot.self,
            from: JSONEncoder().encode(persisted)
        )
        let plan = try XCTUnwrap(
            GaryxSyntheticReservationRecoveryPlanner.plan(
                snapshot: relaunched,
                ledgerKey: ledger.key,
                mergeGeneration: 12
            )
        )

        let fake = GaryxFakeComposerDurabilityStore(initial: relaunched)
        let recovered = try await fake.commit(plan.transaction)
        let recoveredEntry = try XCTUnwrap(recovered.payloadStore.entry(entry.id, scope: scope))
        XCTAssertEqual(recoveredEntry.attachments[envelopeAttachment.id]?.generation, 12)
        XCTAssertEqual(recoveredEntry.attachments[followupAttachment.id]?.generation, 12)
        let recoveredBarrier = try XCTUnwrap(recovered.barriers[entry.id])
        XCTAssertEqual(recoveredBarrier.envelopeAttachmentIDs, [envelopeAttachment.id])
        XCTAssertFalse(recoveredBarrier.envelopeAttachmentIDs.contains(followupAttachment.id))
        XCTAssertEqual(recoveredBarrier.provisionalFollowupAttachmentIDs, [followupAttachment.id])
        XCTAssertTrue(recovered.deliveries.isEmpty, "synthetic revoke never creates S1 outbox payload")
    }

    func testSyntheticRecoveryBeforeProducerDrainedCommitFallsBackToSealedEnvelope() async throws {
        let entry = makeEntry(text: "T")
        var payloadStore = GaryxComposerPayloadStore()
        XCTAssertTrue(payloadStore.insert(entry))
        let ledger = makeLedger(outcome: nil)
        var barrier = makeBarrier()
        XCTAssertEqual(
            barrier.seal(
                reservationID: reservation,
                envelope: makeEnvelope(text: "T"),
                followupGeneration: 11,
                readiness: .ready,
                quota: .init(),
                producerPhase: .live,
                lifecycle: entry.lifecycle.snapshot
            ),
            .sealed
        )
        let initial = GaryxComposerDurabilitySnapshot(
            payloadStore: payloadStore,
            barriers: [entry.id: barrier],
            ledgers: [ledger.key: ledger],
            generationHighWatermark: 32,
            reservationHighWatermark: 32
        )
        let plan = try XCTUnwrap(
            GaryxSyntheticReservationRecoveryPlanner.plan(
                snapshot: initial,
                ledgerKey: ledger.key,
                mergeGeneration: 12
            )
        )

        let fake = GaryxFakeComposerDurabilityStore(initial: initial)
        let recovered = try await fake.commit(plan.transaction)
        XCTAssertEqual(recovered.payloadStore.entry(entry.id, scope: scope)?.currentText, "T")
        XCTAssertEqual(recovered.payloadStore.entry(entry.id, scope: scope)?.currentGeneration, 12)
        XCTAssertEqual(recovered.ledgers[ledger.key]?.terminalOutcome, .revoked)
        XCTAssertEqual(recovered.ledgers[ledger.key]?.targetMapping?.generation, 12)
        XCTAssertTrue(recovered.producerDrained.isEmpty)
        XCTAssertTrue(
            recovered.recoveredInputClosures.isEmpty,
            "without a durable producerDrained record the provisional follow-up may be lost"
        )
    }

    func testNilReservationReplacementDoesNotRequireSendLedger() async throws {
        let fake = GaryxFakeComposerDurabilityStore()
        let replacement = makeReplacement(reservationID: nil)
        let snapshot = try await fake.commit(
            .init(
                expectedRevision: 0,
                label: "ordinary reattach",
                mutations: [.upsertReplacement(replacement)]
            )
        )
        XCTAssertEqual(snapshot.replacements[replacement.id], replacement)
    }

    func testGenerationResetAtomicallyCancelsOperationAndRejectsLateCompletion() async throws {
        var entry = makeEntry(text: "draft")
        let assetID = GaryxStagedAssetID(rawValue: "reset-asset")
        let attachmentID = GaryxAttachmentID(rawValue: "reset-attachment")
        entry.addAttachment(
            GaryxComposerAttachment(
                id: attachmentID,
                stagedAssetID: assetID,
                generation: 10,
                byteCount: 64
            )
        )
        let key = GaryxOperationCapabilityKey(
            scope: scope,
            entryID: entry.id,
            generation: 10,
            reservationID: nil,
            branch: .followup,
            operationID: GaryxOperationID(rawValue: "reset-operation")
        )
        entry.addOperation(key)
        let operation = GaryxOperationCapability(
            context: GaryxScopeBoundOperationContext(
                key: key,
                clientIdentity: "client",
                configurationFingerprint: "configuration",
                payloadLifecycle: GaryxPayloadLifecycleCapture(
                    token: entry.lifecycle.token,
                    revision: entry.lifecycle.revision
                )
            ),
            state: .uploading,
            stagedAssetID: assetID,
            reservedBytes: 64
        )
        let manifest = GaryxOperationManifest(
            key: key,
            stagedPath: "staging/reset-asset.bin",
            state: .uploading,
            uploadAttempted: false
        )
        var payloadStore = GaryxComposerPayloadStore()
        XCTAssertTrue(payloadStore.insert(entry))
        let initial = GaryxComposerDurabilitySnapshot(
            payloadStore: payloadStore,
            operations: [key: operation],
            manifests: [key: manifest],
            stagedAssetOwners: [assetID: key],
            stagedAssetReservedBytes: [assetID: 64],
            reservedBytes: 64,
            generationHighWatermark: 32
        )
        let plan = try XCTUnwrap(
            GaryxPayloadGenerationResetPlanner.plan(
                snapshot: initial,
                scope: scope,
                entryID: entry.id,
                generation: 10,
                allocatedGeneration: 12,
                producerLive: true
            )
        )
        XCTAssertEqual(plan.cancelledOperationKeys, [key])
        XCTAssertEqual(plan.pendingFileCleanupAssetIDs, [assetID])

        for failpoint in plan.transaction.mutations.indices {
            let isolated = GaryxFakeComposerDurabilityStore(initial: initial)
            await isolated.injectFailure(atMutationIndex: failpoint)
            do {
                _ = try await isolated.commit(plan.transaction)
                XCTFail("expected reset failpoint \(failpoint)")
            } catch {
                XCTAssertEqual(
                    error as? GaryxComposerDurabilityError,
                    .injectedFailure(mutationIndex: failpoint)
                )
            }
            let unchanged = try await isolated.load()
            XCTAssertEqual(unchanged, initial)
        }

        let fake = GaryxFakeComposerDurabilityStore(initial: initial)
        let reset = try await fake.commit(plan.transaction)
        let resetEntry = try XCTUnwrap(reset.payloadStore.entry(entry.id, scope: scope))
        XCTAssertEqual(resetEntry.currentGeneration, 12)
        XCTAssertEqual(resetEntry.currentText, "")
        XCTAssertNil(resetEntry.attachments[attachmentID])
        XCTAssertFalse(resetEntry.operationKeys.contains(key))
        XCTAssertNil(reset.operations[key])
        XCTAssertNil(reset.manifests[key])
        XCTAssertNil(reset.stagedAssetOwners[assetID])
        XCTAssertEqual(reset.pendingFileCleanup[assetID], key)
        XCTAssertEqual(reset.reservedBytes, 0)
        XCTAssertEqual(reset.claimedGenerations, [12])

        let scopes = GaryxGatewayScopeRegistry(initialActiveScope: scope)
        var lateCompletion = operation
        XCTAssertEqual(
            lateCompletion.complete(
                expectedKey: key,
                authoritativeEntry: resetEntry,
                lifecycle: resetEntry.lifecycle.snapshot,
                scopes: scopes
            ),
            .archivedIdentityInvalid
        )
        XCTAssertEqual(lateCompletion.state, .cancelled)
        XCTAssertNil(lateCompletion.stagedAssetID)
        XCTAssertEqual(lateCompletion.reservedBytes, 0)

        var lateTransportAttempt = operation
        XCTAssertEqual(
            lateTransportAttempt.markUploadAttempted(
                expectedKey: key,
                authoritativeEntry: resetEntry,
                lifecycle: resetEntry.lifecycle.snapshot,
                scopes: scopes
            ),
            .archivedIdentityInvalid
        )
        XCTAssertFalse(lateTransportAttempt.uploadAttempted)
    }

    func testGenerationResetWithoutOperationsStillClaimsDurableHiLoIdentity() async throws {
        let entry = makeEntry(text: "draft")
        var payloadStore = GaryxComposerPayloadStore()
        XCTAssertTrue(payloadStore.insert(entry))
        let initial = GaryxComposerDurabilitySnapshot(
            payloadStore: payloadStore,
            generationHighWatermark: 32
        )
        let plan = try XCTUnwrap(
            GaryxPayloadGenerationResetPlanner.plan(
                snapshot: initial,
                scope: scope,
                entryID: entry.id,
                generation: 10,
                allocatedGeneration: 11,
                producerLive: true
            )
        )
        XCTAssertTrue(plan.pendingFileCleanupAssetIDs.isEmpty)
        XCTAssertTrue(plan.cancelledOperationKeys.isEmpty)

        let fake = GaryxFakeComposerDurabilityStore(initial: initial)
        let reset = try await fake.commit(plan.transaction)
        let resetEntry = try XCTUnwrap(reset.payloadStore.entry(entry.id, scope: scope))
        XCTAssertEqual(resetEntry.currentGeneration, 11)
        XCTAssertEqual(resetEntry.currentText, "")
        XCTAssertEqual(resetEntry.lifecycle.token, entry.lifecycle.token)
        XCTAssertEqual(reset.claimedGenerations, [11])
    }

    func testDurabilityRejectsStaleOperationAndManifestWithoutEntryMembership() async throws {
        var staleEntry = makeEntry(text: "before reset")
        let key = GaryxOperationCapabilityKey(
            scope: scope,
            entryID: staleEntry.id,
            generation: 10,
            reservationID: nil,
            branch: .followup,
            operationID: GaryxOperationID(rawValue: "stale-operation")
        )
        staleEntry.addOperation(key)
        var staleOperation = GaryxOperationCapability(
            context: GaryxScopeBoundOperationContext(
                key: key,
                clientIdentity: "client",
                configurationFingerprint: "configuration",
                payloadLifecycle: GaryxPayloadLifecycleCapture(
                    token: staleEntry.lifecycle.token,
                    revision: staleEntry.lifecycle.revision
                )
            ),
            state: .uploading
        )
        var currentEntry = staleEntry
        currentEntry.removeOperation(key)
        XCTAssertTrue(
            currentEntry.resetGeneration(
                10,
                to: 11,
                barrierIdle: true,
                producerLive: true
            )
        )
        XCTAssertEqual(
            staleOperation.complete(
                expectedKey: key,
                authoritativeEntry: staleEntry,
                lifecycle: currentEntry.lifecycle.snapshot,
                scopes: GaryxGatewayScopeRegistry(initialActiveScope: scope)
            ),
            .applied,
            "the stale value alone cannot prove it is still authoritative"
        )
        let manifest = GaryxOperationManifest(
            key: key,
            stagedPath: "staging/stale-operation.bin",
            state: .uploading,
            uploadAttempted: true
        )
        var payloadStore = GaryxComposerPayloadStore()
        XCTAssertTrue(payloadStore.insert(currentEntry))
        let initial = GaryxComposerDurabilitySnapshot(payloadStore: payloadStore)

        for mutation in [
            GaryxComposerDurabilityMutation.upsertOperation(staleOperation),
            .upsertManifest(manifest),
        ] {
            let fake = GaryxFakeComposerDurabilityStore(initial: initial)
            do {
                _ = try await fake.commit(
                    .init(expectedRevision: 0, label: "reject stale descendant", mutations: [mutation])
                )
                XCTFail("durability must reject descendants absent from Entry membership")
            } catch let error as GaryxComposerDurabilityError {
                guard case .invariantViolation = error else {
                    return XCTFail("unexpected error \(error)")
                }
            }
            let unchanged = try await fake.load()
            XCTAssertEqual(unchanged, initial)
        }

        let mixedSnapshot = GaryxFakeComposerDurabilityStore(initial: initial)
        do {
            _ = try await mixedSnapshot.commit(
                .init(
                    expectedRevision: 0,
                    label: "reject mixed pre-reset Entry",
                    mutations: [
                        .upsertEntry(staleEntry),
                        .upsertOperation(staleOperation),
                    ]
                )
            )
            XCTFail("an old Entry cannot regress the reset generation to restore membership")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected error \(error)")
            }
        }
        let unchangedMixedSnapshot = try await mixedSnapshot.load()
        XCTAssertEqual(unchangedMixedSnapshot, initial)

        var danglingEntry = currentEntry
        danglingEntry.addOperation(key)
        let danglingMembership = GaryxFakeComposerDurabilityStore(initial: initial)
        do {
            _ = try await danglingMembership.commit(
                .init(
                    expectedRevision: 0,
                    label: "reject dangling Entry membership",
                    mutations: [.upsertEntry(danglingEntry)]
                )
            )
            XCTFail("Entry membership and its durable descendant must publish together")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected error \(error)")
            }
        }
        let unchangedDanglingMembership = try await danglingMembership.load()
        XCTAssertEqual(unchangedDanglingMembership, initial)
    }

    func testGenerationResetRetainsCanonicalFileCleanupUntilPhysicalDeletion() async throws {
        var entry = makeEntry(text: "draft")
        let key = GaryxOperationCapabilityKey(
            scope: scope,
            entryID: entry.id,
            generation: 10,
            reservationID: nil,
            branch: .followup,
            operationID: GaryxOperationID(rawValue: "terminal-reset")
        )
        let assetID = GaryxStagedAssetID(rawValue: "terminal-reset-asset")
        let feedbackID = GaryxFeedbackID(rawValue: "terminal-reset-feedback")
        entry.addOperation(key)
        entry.addFeedbackReference(feedbackID)
        let operation = GaryxOperationCapability(
            context: GaryxScopeBoundOperationContext(
                key: key,
                clientIdentity: "client",
                configurationFingerprint: "configuration",
                payloadLifecycle: GaryxPayloadLifecycleCapture(
                    token: entry.lifecycle.token,
                    revision: entry.lifecycle.revision
                )
            ),
            state: .failedTerminal,
            stagedAssetID: assetID,
            reservedBytes: 64
        )
        let manifest = GaryxOperationManifest(
            key: key,
            stagedPath: "staging/terminal-reset.bin",
            state: .failedTerminal,
            uploadAttempted: true
        )
        let feedback = GaryxOperationFeedback(
            id: feedbackID,
            scope: scope,
            entryID: entry.id,
            operationID: key.operationID,
            kind: .uploadTerminal
        )
        var payloadStore = GaryxComposerPayloadStore()
        XCTAssertTrue(payloadStore.insert(entry))
        let initial = GaryxComposerDurabilitySnapshot(
            payloadStore: payloadStore,
            operations: [key: operation],
            manifests: [key: manifest],
            feedback: [feedbackID: feedback],
            pendingFileCleanup: [assetID: key],
            generationHighWatermark: 32
        )
        let plan = try XCTUnwrap(
            GaryxPayloadGenerationResetPlanner.plan(
                snapshot: initial,
                scope: scope,
                entryID: entry.id,
                generation: 10,
                allocatedGeneration: 11,
                producerLive: true
            )
        )
        XCTAssertEqual(plan.pendingFileCleanupAssetIDs, [assetID])

        for failpoint in plan.transaction.mutations.indices {
            let isolated = GaryxFakeComposerDurabilityStore(initial: initial)
            await isolated.injectFailure(atMutationIndex: failpoint)
            do {
                _ = try await isolated.commit(plan.transaction)
                XCTFail("expected canonical cleanup reset failpoint \(failpoint)")
            } catch {
                XCTAssertEqual(
                    error as? GaryxComposerDurabilityError,
                    .injectedFailure(mutationIndex: failpoint)
                )
            }
            let unchanged = try await isolated.load()
            XCTAssertEqual(unchanged, initial)
        }

        let fake = GaryxFakeComposerDurabilityStore(initial: initial)
        let reset = try await fake.commit(plan.transaction)
        XCTAssertEqual(reset.pendingFileCleanup[assetID], key)
        XCTAssertNil(reset.operations[key])
        XCTAssertNil(reset.manifests[key])
        XCTAssertEqual(reset.feedback[feedbackID]?.phase, .archived)
        XCTAssertFalse(
            try XCTUnwrap(reset.payloadStore.entry(entry.id, scope: scope))
                .feedbackReferences.contains(feedbackID)
        )

        let encoded = try JSONEncoder().encode(reset)
        let relaunchedSnapshot = try JSONDecoder().decode(
            GaryxComposerDurabilitySnapshot.self,
            from: encoded
        )
        XCTAssertEqual(relaunchedSnapshot.pendingFileCleanup[assetID], key)

        let relaunched = GaryxFakeComposerDurabilityStore(initial: relaunchedSnapshot)
        let physicallyDeleted = try await relaunched.commit(
            .init(
                expectedRevision: relaunchedSnapshot.revision,
                label: "physical deletion acknowledged after reset",
                mutations: [.completeFileCleanup(assetID)]
            )
        )
        XCTAssertNil(physicallyDeleted.pendingFileCleanup[assetID])
    }

    func testDiscardTombstoneRejectsReinsertingStaleEntryAndOperation() async throws {
        var staleEntry = makeEntry(text: "stale payload")
        let staleOperation = makeOperation(state: .uploading)
        staleEntry.addOperation(staleOperation.context.key)
        var payloadStore = GaryxComposerPayloadStore()
        XCTAssertTrue(payloadStore.insert(staleEntry))
        let initial = GaryxComposerDurabilitySnapshot(
            payloadStore: payloadStore,
            operations: [staleOperation.context.key: staleOperation]
        )

        var convergence = makeDiscardConvergence()
        convergence.settleDeliveries()
        convergence.settleReservation()
        convergence.settleSessions()
        convergence.settleResources()
        XCTAssertTrue(convergence.finishToken())

        let fake = GaryxFakeComposerDurabilityStore(initial: initial)
        let discarded = try await fake.commit(
            .init(
                expectedRevision: initial.revision,
                label: "finish discard and remove authoritative payload",
                mutations: [
                    .upsertDiscardConvergence(convergence),
                    .removeOperation(staleOperation.context.key),
                    .removeEntry(scope: scope, entryID: staleEntry.id),
                ]
            )
        )
        XCTAssertEqual(discarded.discardConvergence[staleEntry.id]?.lifecycle.phase, .discarded)
        XCTAssertNil(discarded.payloadStore.entry(staleEntry.id, scope: scope))

        do {
            _ = try await fake.commit(
                .init(
                    expectedRevision: discarded.revision,
                    label: "reject stale callback resurrection",
                    mutations: [
                        .upsertEntry(staleEntry),
                        .upsertOperation(staleOperation),
                    ]
                )
            )
            XCTFail("discard tombstone must reject reinserting its retired payload identity")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected error \(error)")
            }
        }
        let unchanged = try await fake.load()
        XCTAssertEqual(unchanged, discarded)
    }

    func testGenerationResetRetainsAbortedReplacementUntilProvisionalCleanupSettles() async throws {
        var entry = makeEntry(text: "draft")
        let key = GaryxOperationCapabilityKey(
            scope: scope,
            entryID: entry.id,
            generation: 10,
            reservationID: nil,
            branch: .followup,
            operationID: GaryxOperationID(rawValue: "replacement-old")
        )
        entry.addOperation(key)
        let operation = GaryxOperationCapability(
            context: GaryxScopeBoundOperationContext(
                key: key,
                clientIdentity: "client",
                configurationFingerprint: "configuration",
                payloadLifecycle: GaryxPayloadLifecycleCapture(
                    token: entry.lifecycle.token,
                    revision: entry.lifecycle.revision
                )
            ),
            state: .failedRetryable
        )
        let replacement = GaryxReplacementRecord(
            id: GaryxReplacementID(rawValue: "reset-pending-replacement"),
            scope: scope,
            entryID: entry.id,
            oldKey: key,
            reservationID: nil,
            branch: .followup,
            stagedAssetID: GaryxStagedAssetID(rawValue: "reset-provisional-file"),
            reservedBytes: 128
        )
        var payloadStore = GaryxComposerPayloadStore()
        XCTAssertTrue(payloadStore.insert(entry))
        let initial = GaryxComposerDurabilitySnapshot(
            payloadStore: payloadStore,
            operations: [key: operation],
            replacements: [replacement.id: replacement],
            generationHighWatermark: 32
        )
        let plan = try XCTUnwrap(
            GaryxPayloadGenerationResetPlanner.plan(
                snapshot: initial,
                scope: scope,
                entryID: entry.id,
                generation: 10,
                allocatedGeneration: 11,
                producerLive: true
            )
        )

        for failpoint in plan.transaction.mutations.indices {
            let isolated = GaryxFakeComposerDurabilityStore(initial: initial)
            await isolated.injectFailure(atMutationIndex: failpoint)
            do {
                _ = try await isolated.commit(plan.transaction)
                XCTFail("expected replacement cleanup reset failpoint \(failpoint)")
            } catch {
                XCTAssertEqual(
                    error as? GaryxComposerDurabilityError,
                    .injectedFailure(mutationIndex: failpoint)
                )
            }
            let unchanged = try await isolated.load()
            XCTAssertEqual(unchanged, initial)
        }

        let fake = GaryxFakeComposerDurabilityStore(initial: initial)
        let reset = try await fake.commit(plan.transaction)
        XCTAssertEqual(plan.pendingReplacementCleanupIDs, [replacement.id])
        let cleanupRecord = try XCTUnwrap(reset.replacements[replacement.id])
        XCTAssertEqual(cleanupRecord.phase, .aborted)
        XCTAssertEqual(
            GaryxReplacementPlanner.recover(cleanupRecord),
            .abortReleaseQuotaAndDeleteProvisional
        )
        XCTAssertEqual(cleanupRecord.stagedAssetID, replacement.stagedAssetID)
        XCTAssertEqual(cleanupRecord.reservedBytes, replacement.reservedBytes)

        var settledRecord = cleanupRecord
        settledRecord.settle()
        let settled = try await fake.commit(
            .init(
                expectedRevision: reset.revision,
                label: "settle provisional replacement cleanup",
                mutations: [
                    .upsertReplacement(settledRecord),
                    .removeReplacement(settledRecord.id),
                ]
            )
        )
        XCTAssertNil(settled.replacements[replacement.id])
    }

    func testOperationRemovalAndFeedbackAcknowledgementPublishAsOneTransaction() async throws {
        var entry = makeEntry(text: "surviving text")
        let key = GaryxOperationCapabilityKey(
            scope: scope,
            entryID: entry.id,
            generation: 10,
            reservationID: nil,
            branch: .followup,
            operationID: GaryxOperationID(rawValue: "terminal-remove")
        )
        let assetID = GaryxStagedAssetID(rawValue: "terminal-remove-asset")
        let feedbackID = GaryxFeedbackID(rawValue: "terminal-remove-feedback")
        let lineageID = GaryxAttachmentLineageID(rawValue: "terminal-remove-lineage")
        entry.addOperation(key)
        entry.addFeedbackReference(feedbackID)
        let operation = GaryxOperationCapability(
            context: GaryxScopeBoundOperationContext(
                key: key,
                clientIdentity: "client",
                configurationFingerprint: "configuration",
                payloadLifecycle: GaryxPayloadLifecycleCapture(
                    token: entry.lifecycle.token,
                    revision: entry.lifecycle.revision
                )
            ),
            state: .failedTerminal,
            stagedAssetID: assetID,
            reservedBytes: 48
        )
        let manifest = GaryxOperationManifest(
            key: key,
            stagedPath: "staging/terminal-remove.bin",
            state: .failedTerminal,
            uploadAttempted: true
        )
        let feedback = GaryxOperationFeedback(
            id: feedbackID,
            scope: scope,
            entryID: entry.id,
            operationID: key.operationID,
            lineageID: lineageID,
            kind: .uploadTerminal
        )
        let lineage = GaryxAttachmentLineageTombstone(
            id: lineageID,
            scope: scope,
            entryID: entry.id,
            attachmentSlotID: GaryxAttachmentID(rawValue: "terminal-remove-slot"),
            failedOperationID: key.operationID,
            feedbackID: feedbackID,
            payloadLifecycle: operation.context.payloadLifecycle
        )
        var payloadStore = GaryxComposerPayloadStore()
        XCTAssertTrue(payloadStore.insert(entry))
        let initial = GaryxComposerDurabilitySnapshot(
            payloadStore: payloadStore,
            operations: [key: operation],
            manifests: [key: manifest],
            feedback: [feedbackID: feedback],
            attachmentLineages: [lineageID: lineage],
            stagedAssetOwners: [assetID: key],
            stagedAssetReservedBytes: [assetID: 48],
            pendingFileCleanup: [assetID: key],
            reservedBytes: 48
        )
        let plan = try XCTUnwrap(
            GaryxOperationRemovalFeedbackPlanner.plan(
                snapshot: initial,
                operationKey: key,
                feedbackID: feedbackID,
                scopes: GaryxGatewayScopeRegistry(initialActiveScope: scope)
            )
        )
        XCTAssertEqual(plan.pendingFileCleanupAssetID, assetID)

        for failpoint in plan.transaction.mutations.indices {
            let isolated = GaryxFakeComposerDurabilityStore(initial: initial)
            await isolated.injectFailure(atMutationIndex: failpoint)
            do {
                _ = try await isolated.commit(plan.transaction)
                XCTFail("expected remove-action failpoint \(failpoint)")
            } catch {
                XCTAssertEqual(
                    error as? GaryxComposerDurabilityError,
                    .injectedFailure(mutationIndex: failpoint)
                )
            }
            let unchanged = try await isolated.load()
            XCTAssertEqual(unchanged, initial)
        }

        let fake = GaryxFakeComposerDurabilityStore(initial: initial)
        let removed = try await fake.commit(plan.transaction)
        let survivingEntry = try XCTUnwrap(removed.payloadStore.entry(entry.id, scope: scope))
        XCTAssertEqual(survivingEntry.currentText, "surviving text")
        XCTAssertFalse(survivingEntry.operationKeys.contains(key))
        XCTAssertFalse(survivingEntry.feedbackReferences.contains(feedbackID))
        XCTAssertNil(removed.operations[key])
        XCTAssertNil(removed.manifests[key])
        XCTAssertNil(removed.stagedAssetOwners[assetID])
        XCTAssertEqual(removed.pendingFileCleanup[assetID], key)
        XCTAssertEqual(removed.reservedBytes, 0)
        XCTAssertEqual(removed.feedback[feedbackID]?.phase, .acknowledged)
        XCTAssertEqual(removed.attachmentLineages[lineageID]?.phase, .released)

        let physicallyDeleted = try await fake.commit(
            .init(
                expectedRevision: removed.revision,
                label: "physical deletion acknowledged after remove action",
                mutations: [.completeFileCleanup(assetID)]
            )
        )
        XCTAssertNil(physicallyDeleted.pendingFileCleanup[assetID])
    }

    func testSealedBarrierRequiresMatchingReservationLedgerFirst() async throws {
        let entry = makeEntry(text: "draft")
        var barrier = makeBarrier()
        XCTAssertEqual(
            barrier.seal(
                reservationID: reservation,
                envelope: makeEnvelope(text: "draft"),
                followupGeneration: 11,
                readiness: .ready,
                quota: .init(),
                producerPhase: .live,
                lifecycle: entry.lifecycle.snapshot
            ),
            .sealed
        )
        let missingLedger = GaryxFakeComposerDurabilityStore()
        do {
            _ = try await missingLedger.commit(
                .init(
                    expectedRevision: 0,
                    label: "barrier without ledger",
                    mutations: [.upsertBarrier(barrier)]
                )
            )
            XCTFail("expected ledger-first rejection")
        } catch {
            XCTAssertEqual(
                error as? GaryxComposerDurabilityError,
                .invariantViolation("send barrier requires reservation ledger first")
            )
        }
        let unchanged = try await missingLedger.load()
        XCTAssertEqual(unchanged, GaryxComposerDurabilitySnapshot())

        let ordered = GaryxFakeComposerDurabilityStore()
        let committed = try await ordered.commit(
            .init(
                expectedRevision: 0,
                label: "ledger then barrier",
                mutations: [.upsertLedger(makeLedger(outcome: nil)), .upsertBarrier(barrier)]
            )
        )
        XCTAssertEqual(committed.barriers[entry.id], barrier)
    }

    func testCommitSendThreeInOneTransactionPublishesLedgerPayloadAndDeliveryTogether() async throws {
        var ledger = makeLedger(outcome: nil)
        ledger.settle(.committed, targetGeneration: 11)
        var barrier = makeBarrier()
        let entry = makeEntry(text: "T")
        _ = barrier.seal(
            reservationID: reservation,
            envelope: makeEnvelope(text: "T"),
            followupGeneration: 11,
            readiness: .ready,
            quota: .init(),
            producerPhase: .live,
            lifecycle: entry.lifecycle.snapshot
        )
        XCTAssertTrue(
            barrier.replaceProvisionalText("U", lifecycle: entry.lifecycle.snapshot)
        )
        let settlement = try XCTUnwrap(
            barrier.durableCommit(
                deliveryID: deliveryID,
                correlationID: "correlation",
                clientIntentID: "intent",
                lifecycle: entry.lifecycle.snapshot
            )
        )
        let send = try GaryxComposerCommitSend(
            expectedRevision: 0,
            ledger: ledger,
            sealedPayloadEntry: entry,
            barrier: barrier,
            settlement: settlement
        )
        let delivery = send.delivery

        let fake = GaryxFakeComposerDurabilityStore()
        for failpoint in send.transaction.mutations.indices {
            let isolated = GaryxFakeComposerDurabilityStore()
            await isolated.injectFailure(atMutationIndex: failpoint)
            do {
                _ = try await isolated.commitSend(send)
                XCTFail("expected failure")
            } catch {}
            let unchanged = try await isolated.load()
            XCTAssertEqual(unchanged, GaryxComposerDurabilitySnapshot())
        }

        let snapshot = try await fake.commitSend(send)
        XCTAssertEqual(snapshot.ledgers[ledger.key]?.terminalOutcome, .committed)
        XCTAssertEqual(snapshot.payloadStore.entry(entry.id, scope: scope)?.currentText, "U")
        XCTAssertNil(snapshot.payloadStore.entry(entry.id, scope: scope)?.textByGeneration[10])
        XCTAssertEqual(snapshot.payloadStore.entry(entry.id, scope: scope)?.currentGeneration, 11)
        XCTAssertTrue(
            snapshot.payloadStore.entry(entry.id, scope: scope)?.deliveryReferences.contains(delivery.id)
                == true
        )
        XCTAssertEqual(snapshot.barriers[entry.id]?.phase, .durableCommitted)
        XCTAssertEqual(snapshot.deliveries[delivery.id], delivery)
    }

    func testCommittedSendRelaunchKeepsEnvelopeAttachmentOutOfFollowupPayload() async throws {
        let envelopeAttachmentID = GaryxAttachmentID(rawValue: "committed-envelope-attachment")
        let followupAttachmentID = GaryxAttachmentID(rawValue: "committed-followup-attachment")
        var entry = makeEntry(text: "T")
        entry.addAttachment(
            GaryxComposerAttachment(
                id: envelopeAttachmentID,
                stagedAssetID: GaryxStagedAssetID(rawValue: "committed-envelope-asset"),
                generation: 10,
                byteCount: 40
            )
        )
        var barrier = makeBarrier()
        XCTAssertEqual(
            barrier.seal(
                reservationID: reservation,
                envelope: makeEnvelope(text: "T", attachments: [envelopeAttachmentID]),
                followupGeneration: 11,
                readiness: .ready,
                quota: .init(),
                producerPhase: .live,
                lifecycle: entry.lifecycle.snapshot
            ),
            .sealed
        )
        XCTAssertTrue(barrier.replaceProvisionalText("U", lifecycle: entry.lifecycle.snapshot))
        XCTAssertTrue(
            barrier.addProvisionalAttachment(
                followupAttachmentID,
                lifecycle: entry.lifecycle.snapshot
            )
        )
        let settlement = try XCTUnwrap(
            barrier.durableCommit(
                deliveryID: deliveryID,
                correlationID: "committed-attachment-correlation",
                clientIntentID: "intent",
                lifecycle: entry.lifecycle.snapshot
            )
        )
        let delivery = try XCTUnwrap(settlement.deliveryRecord)
        var ledger = makeLedger(outcome: nil)
        ledger.settle(.committed, targetGeneration: settlement.followupGeneration)

        entry.removeAttachment(envelopeAttachmentID)
        entry.addAttachment(
            GaryxComposerAttachment(
                id: followupAttachmentID,
                stagedAssetID: GaryxStagedAssetID(rawValue: "committed-followup-asset"),
                generation: settlement.followupGeneration,
                byteCount: 20
            )
        )
        entry.setText(settlement.followupText, generation: settlement.followupGeneration)

        let fake = GaryxFakeComposerDurabilityStore()
        let committed = try await fake.commit(
            .init(
                expectedRevision: 0,
                label: "commit send before simulated process death",
                mutations: [
                    .upsertLedger(ledger),
                    .upsertEntry(entry),
                    .upsertBarrier(barrier),
                    .upsertDelivery(delivery),
                ]
            )
        )
        let relaunched = try JSONDecoder().decode(
            GaryxComposerDurabilitySnapshot.self,
            from: JSONEncoder().encode(committed)
        )
        let restoredEntry = try XCTUnwrap(
            relaunched.payloadStore.entry(entry.id, scope: scope)
        )
        let restoredEnvelope = try XCTUnwrap(relaunched.deliveries[delivery.id]?.envelope)

        XCTAssertEqual(relaunched.ledgers[ledger.key]?.terminalOutcome, .committed)
        XCTAssertEqual(restoredEnvelope.attachmentIDs, [envelopeAttachmentID])
        XCTAssertFalse(restoredEnvelope.attachmentIDs.contains(followupAttachmentID))
        XCTAssertNil(restoredEntry.attachments[envelopeAttachmentID])
        XCTAssertEqual(restoredEntry.attachments[followupAttachmentID]?.generation, 11)
        XCTAssertEqual(restoredEntry.currentText, "U")
    }

    func testAmbiguousRestoreAndConflictMembershipCommitAtomically() async throws {
        let ledger = makeLedger(outcome: .committed)
        var original = GaryxDeliveryRecord(
            id: deliveryID,
            scope: scope,
            entryID: entryID,
            reservationID: reservation,
            correlationID: "correlation",
            envelope: makeEnvelope(text: "message")
        )
        XCTAssertTrue(original.markTransportAttempted())
        XCTAssertTrue(original.markAmbiguous())
        var recovered = original
        var conflict = GaryxPayloadConflictSet(
            id: GaryxPayloadConflictSetID(rawValue: "delivery-conflict"),
            scope: scope
        )
        let candidate = GaryxPayloadConflictCandidate(
            entryID: entryID,
            label: "Recovered draft"
        )
        guard case .restored = GaryxDeliveryDraftRecoveryReducer.restore(
            record: &recovered,
            conflictSet: &conflict,
            candidate: candidate,
            membershipDurabilityAvailable: true
        ) else {
            return XCTFail("expected recovery plan")
        }

        let initial = GaryxComposerDurabilitySnapshot(
            ledgers: [ledger.key: ledger],
            deliveries: [original.id: original]
        )
        let mutations: [GaryxComposerDurabilityMutation] = [
            .upsertConflict(conflict),
            .upsertDelivery(recovered),
        ]
        for index in mutations.indices {
            let fake = GaryxFakeComposerDurabilityStore(initial: initial)
            await fake.injectFailure(atMutationIndex: index)
            do {
                _ = try await fake.commit(
                    .init(expectedRevision: 0, label: "restore ambiguous", mutations: mutations)
                )
                XCTFail("expected failpoint")
            } catch {}
            let unchanged = try await fake.load()
            XCTAssertEqual(unchanged, initial)
        }

        let fake = GaryxFakeComposerDurabilityStore(initial: initial)
        let committed = try await fake.commit(
            .init(expectedRevision: 0, label: "restore ambiguous", mutations: mutations)
        )
        XCTAssertEqual(committed.deliveries[original.id]?.phase, .abandoned)
        XCTAssertEqual(committed.conflicts[conflict.id]?.candidates, [candidate])
    }

    func testDeliveryUpsertRejectsAcknowledgementEvidenceRegression() async throws {
        let ledger = makeLedger(outcome: .committed)
        var attempted = makeDelivery()
        XCTAssertTrue(attempted.markTransportAttempted())
        var acknowledged = attempted
        acknowledged.recordServerAcknowledgement()
        let initial = GaryxComposerDurabilitySnapshot(
            ledgers: [ledger.key: ledger],
            deliveries: [acknowledged.id: acknowledged]
        )
        let fake = GaryxFakeComposerDurabilityStore(initial: initial)

        do {
            _ = try await fake.commit(
                .init(
                    expectedRevision: 0,
                    label: "reject stale delivery overwrite",
                    mutations: [.upsertDelivery(attempted)]
                )
            )
            XCTFail("server acknowledgement must not regress to attempted evidence")
        } catch {
            XCTAssertEqual(
                error as? GaryxComposerDurabilityError,
                .invariantViolation("delivery phase or evidence regressed")
            )
        }
        let unchanged = try await fake.load()
        XCTAssertEqual(unchanged, initial)
    }

    func testDeliveryUpsertRejectsEnvelopeResurrection() async throws {
        let ledger = makeLedger(outcome: .committed)
        let original = makeDelivery()
        var settled = original
        settled.settleForDiscard()
        let resurrected = try mutateDeliveryJSON(settled) { object in
            object["envelope"] = try encodedJSONObject(original)["envelope"]
        }
        let initial = GaryxComposerDurabilitySnapshot(
            ledgers: [ledger.key: ledger],
            deliveries: [settled.id: settled]
        )
        let fake = GaryxFakeComposerDurabilityStore(initial: initial)

        do {
            _ = try await fake.commit(
                .init(
                    expectedRevision: 0,
                    label: "reject delivery envelope resurrection",
                    mutations: [.upsertDelivery(resurrected)]
                )
            )
            XCTFail("a cleared send envelope must never be reconstructed")
        } catch {
            XCTAssertEqual(
                error as? GaryxComposerDurabilityError,
                .invariantViolation("delivery phase or evidence regressed")
            )
        }
        let unchanged = try await fake.load()
        XCTAssertEqual(unchanged, initial)
    }

    func testDeliveryDispositionAllowsOnlyPayloadDiscardedToScopeRevokedAdvance() async throws {
        let ledger = makeLedger(outcome: .committed)
        var discarded = makeDelivery()
        discarded.settleForDiscard()
        var revoked = discarded
        revoked.settleForScopeRevoke()
        let initial = GaryxComposerDurabilitySnapshot(
            ledgers: [ledger.key: ledger],
            deliveries: [discarded.id: discarded]
        )
        let fake = GaryxFakeComposerDurabilityStore(initial: initial)
        let advanced = try await fake.commit(
            .init(
                expectedRevision: 0,
                label: "advance discard disposition to scope revoke",
                mutations: [.upsertDelivery(revoked)]
            )
        )
        XCTAssertEqual(
            advanced.deliveries[revoked.id]?.userDisposition,
            .scopeRevoked
        )

        let regressed = try mutateDeliveryJSON(revoked) { object in
            object["userDisposition"] = GaryxDeliveryUserDisposition.payloadDiscarded.rawValue
        }
        do {
            _ = try await fake.commit(
                .init(
                    expectedRevision: advanced.revision,
                    label: "reject scope revoke disposition regression",
                    mutations: [.upsertDelivery(regressed)]
                )
            )
            XCTFail("scope-revoked disposition must not regress")
        } catch {
            XCTAssertEqual(
                error as? GaryxComposerDurabilityError,
                .invariantViolation("delivery phase or evidence regressed")
            )
        }
        let unchanged = try await fake.load()
        XCTAssertEqual(unchanged, advanced)
    }

    func testDeliveryUpsertRejectsDuplicateRecordIdentityRewrite() async throws {
        let ledger = makeLedger(outcome: .committed)
        var duplicate = makeDelivery()
        XCTAssertTrue(duplicate.markTransportAttempted())
        XCTAssertTrue(duplicate.markAmbiguous())
        XCTAssertNotNil(
            duplicate.resendAsDuplicate(
                newRecordID: .init(rawValue: "duplicate-record"),
                newClientIntentID: "duplicate-intent"
            )
        )
        let rewritten = try mutateDeliveryJSON(duplicate) { object in
            object["duplicateRecordID"] = try encodedJSONValue(
                GaryxDeliveryRecordID(rawValue: "different-duplicate-record")
            )
        }
        let initial = GaryxComposerDurabilitySnapshot(
            ledgers: [ledger.key: ledger],
            deliveries: [duplicate.id: duplicate]
        )
        let fake = GaryxFakeComposerDurabilityStore(initial: initial)

        do {
            _ = try await fake.commit(
                .init(
                    expectedRevision: 0,
                    label: "reject duplicate delivery identity rewrite",
                    mutations: [.upsertDelivery(rewritten)]
                )
            )
            XCTFail("duplicate delivery identity must be immutable once assigned")
        } catch {
            XCTAssertEqual(
                error as? GaryxComposerDurabilityError,
                .invariantViolation("delivery phase or evidence regressed")
            )
        }
        let unchanged = try await fake.load()
        XCTAssertEqual(unchanged, initial)
    }

    func testFailedTerminalFeedbackEntryReferenceAndOperationAreAtomic() async throws {
        var entry = makeEntry()
        let feedback = makeFeedback()
        entry.addFeedbackReference(feedback.id)
        let cleanupAssetID = GaryxStagedAssetID(rawValue: "terminal-cleanup")
        let operation = makeOperation(
            state: .failedTerminal,
            assetID: cleanupAssetID,
            reservedBytes: 64
        )
        entry.addOperation(operation.context.key)
        let mutations: [GaryxComposerDurabilityMutation] = [
            .upsertOperation(operation),
            .upsertFeedback(feedback),
            .upsertEntry(entry),
            .registerFileCleanup(assetID: cleanupAssetID, owner: operation.context.key),
        ]

        for index in mutations.indices {
            let fake = GaryxFakeComposerDurabilityStore()
            await fake.injectFailure(atMutationIndex: index)
            do {
                _ = try await fake.commit(
                    .init(expectedRevision: 0, label: "failed terminal", mutations: mutations)
                )
                XCTFail("expected failpoint")
            } catch {}
            let snapshot = try await fake.load()
            XCTAssertTrue(snapshot.operations.isEmpty)
            XCTAssertTrue(snapshot.feedback.isEmpty)
            XCTAssertNil(snapshot.payloadStore.entry(entry.id, scope: scope))
            XCTAssertTrue(snapshot.pendingFileCleanup.isEmpty)
        }

        let fake = GaryxFakeComposerDurabilityStore()
        let snapshot = try await fake.commit(
            .init(expectedRevision: 0, label: "failed terminal", mutations: mutations)
        )
        XCTAssertEqual(snapshot.operations[operation.context.key]?.state, .failedTerminal)
        XCTAssertEqual(snapshot.feedback[feedback.id]?.phase, .pending)
        XCTAssertTrue(
            snapshot.payloadStore.entry(entry.id, scope: scope)?.feedbackReferences.contains(feedback.id) == true
        )
        XCTAssertEqual(snapshot.pendingFileCleanup[cleanupAssetID], operation.context.key)

        let cleaned = try await fake.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "physical cleanup completed idempotently",
                mutations: [
                    .completeFileCleanup(cleanupAssetID),
                    .completeFileCleanup(cleanupAssetID),
                ]
            )
        )
        XCTAssertTrue(cleaned.pendingFileCleanup.isEmpty)
    }

    func testFreshOperationFeedbackAckAndLineageReleaseAreAtomic() async throws {
        let entry = makeEntry()
        let lineageID = GaryxAttachmentLineageID(rawValue: "lineage")
        let feedbackID = GaryxFeedbackID(rawValue: "lineage-feedback")
        let pendingFeedback = GaryxOperationFeedback(
            id: feedbackID,
            scope: scope,
            entryID: entry.id,
            operationID: GaryxOperationID(rawValue: "failed"),
            lineageID: lineageID,
            kind: .uploadTerminal
        )
        let retainedLineage = GaryxAttachmentLineageTombstone(
            id: lineageID,
            scope: scope,
            entryID: entry.id,
            attachmentSlotID: GaryxAttachmentID(rawValue: "stable-slot"),
            failedOperationID: GaryxOperationID(rawValue: "failed"),
            feedbackID: feedbackID,
            payloadLifecycle: GaryxPayloadLifecycleCapture(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
        )
        var fresh = makeOperation(id: "fresh", state: .requested)
        XCTAssertTrue(
            retainedLineage.admitsFreshOperation(
                fresh,
                feedback: pendingFeedback,
                lifecycle: entry.lifecycle.snapshot
            )
        )

        var acknowledgedFeedback = pendingFeedback
        var releasedLineage = retainedLineage
        XCTAssertEqual(
            GaryxFailedTerminalReattachReducer.commit(
                freshOperation: &fresh,
                feedback: &acknowledgedFeedback,
                lineage: &releasedLineage,
                lifecycle: entry.lifecycle.snapshot,
                scopes: GaryxGatewayScopeRegistry(initialActiveScope: scope)
            ),
            .committed
        )

        var admittedEntry = entry
        admittedEntry.addOperation(fresh.context.key)
        var payloadStore = GaryxComposerPayloadStore()
        XCTAssertTrue(payloadStore.insert(entry))
        let initial = GaryxComposerDurabilitySnapshot(
            payloadStore: payloadStore,
            feedback: [feedbackID: pendingFeedback],
            attachmentLineages: [lineageID: retainedLineage]
        )
        let mutations: [GaryxComposerDurabilityMutation] = [
            .upsertOperation(fresh),
            .upsertFeedback(acknowledgedFeedback),
            .upsertAttachmentLineage(releasedLineage),
            .upsertEntry(admittedEntry),
        ]
        for index in mutations.indices {
            let fake = GaryxFakeComposerDurabilityStore(initial: initial)
            await fake.injectFailure(atMutationIndex: index)
            do {
                _ = try await fake.commit(
                    .init(expectedRevision: 0, label: "fresh reattach", mutations: mutations)
                )
                XCTFail("expected injected failure")
            } catch {}
            let unchanged = try await fake.load()
            XCTAssertEqual(unchanged, initial)
        }

        let fake = GaryxFakeComposerDurabilityStore(initial: initial)
        let committed = try await fake.commit(
            .init(expectedRevision: 0, label: "fresh reattach", mutations: mutations)
        )
        XCTAssertEqual(committed.operations[fresh.context.key], fresh)
        XCTAssertEqual(committed.operations[fresh.context.key]?.state, .preparing)
        XCTAssertEqual(committed.feedback[feedbackID]?.phase, .acknowledged)
        XCTAssertEqual(committed.attachmentLineages[lineageID]?.phase, .released)
    }

    func testConflictAdmissionAndPromotionFailClosedInOneTransaction() async throws {
        var payloadStore = GaryxComposerPayloadStore()
        let entry = makeEntry(text: "draft")
        XCTAssertTrue(payloadStore.insert(entry))
        let initial = GaryxComposerDurabilitySnapshot(payloadStore: payloadStore)
        var promoted = entry
        promoted.promote(to: .thread("thread"))
        var conflict = GaryxPayloadConflictSet(
            id: GaryxPayloadConflictSetID(rawValue: "conflict"),
            scope: scope
        )
        XCTAssertTrue(
            conflict.admitCandidate(
                GaryxPayloadConflictCandidate(entryID: entry.id, label: "Draft"),
                membershipDurabilityAvailable: true
            )
        )

        for index in 0..<2 {
            let fake = GaryxFakeComposerDurabilityStore(initial: initial)
            await fake.injectFailure(atMutationIndex: index)
            do {
                _ = try await fake.commit(
                    .init(
                        expectedRevision: 0,
                        label: "promotion conflict",
                        mutations: [.upsertConflict(conflict), .upsertEntry(promoted)]
                    )
                )
                XCTFail("expected failure")
            } catch {}
            let snapshot = try await fake.load()
            XCTAssertTrue(snapshot.conflicts.isEmpty)
            XCTAssertEqual(snapshot.payloadStore.entry(entry.id, scope: scope)?.destination, .draft("D"))
        }

        let fake = GaryxFakeComposerDurabilityStore(initial: initial)
        let committed = try await fake.commit(
            .init(
                expectedRevision: 0,
                label: "promotion conflict",
                mutations: [.upsertConflict(conflict), .upsertEntry(promoted)]
            )
        )
        XCTAssertEqual(committed.conflicts[conflict.id], conflict)
        XCTAssertEqual(committed.payloadStore.entry(entry.id, scope: scope)?.destination, .thread("thread"))
    }

    func testReplacementSwapAndFileOwnershipCommitAtomically() async throws {
        var old = makeOperation(
            id: "old",
            state: .failedRetryable,
            assetID: GaryxStagedAssetID(rawValue: "old-asset"),
            reservedBytes: 100
        )
        var successor = makeOperation(id: "new", state: .requested)
        var replacement = makeReplacement(oldKey: old.context.key, reservationID: nil)
        var feedback = GaryxOperationFeedback(
            id: GaryxFeedbackID(rawValue: "retry-feedback"),
            scope: scope,
            entryID: entryID,
            operationID: old.context.key.operationID,
            kind: .uploadRetryable
        )
        var entry = makeEntry()
        let scopes = GaryxGatewayScopeRegistry(initialActiveScope: scope)
        XCTAssertEqual(
            GaryxReplacementFeedbackSwapReducer.commit(
                old: &old,
                successor: &successor,
                record: &replacement,
                feedback: &feedback,
                lifecycle: entry.lifecycle.snapshot,
                scopes: scopes
            ),
            .committed
        )
        entry.addOperation(old.context.key)
        entry.addOperation(successor.context.key)
        let mutations: [GaryxComposerDurabilityMutation] = [
            .upsertEntry(entry),
            .upsertOperation(old),
            .upsertOperation(successor),
            .upsertReplacement(replacement),
            .upsertFeedback(feedback),
            .reserveStagedAsset(
                assetID: replacement.stagedAssetID,
                owner: successor.context.key,
                bytes: replacement.reservedBytes
            ),
        ]
        for index in mutations.indices {
            let fake = GaryxFakeComposerDurabilityStore()
            await fake.injectFailure(atMutationIndex: index)
            do {
                _ = try await fake.commit(
                    .init(expectedRevision: 0, label: "replacement swap", mutations: mutations)
                )
                XCTFail("expected failure")
            } catch {}
            let unchanged = try await fake.load()
            XCTAssertEqual(unchanged, GaryxComposerDurabilitySnapshot())
        }

        let fake = GaryxFakeComposerDurabilityStore()
        let snapshot = try await fake.commit(
            .init(expectedRevision: 0, label: "replacement swap", mutations: mutations)
        )
        XCTAssertEqual(snapshot.stagedAssetOwners.count, 1)
        XCTAssertEqual(snapshot.stagedAssetOwners[replacement.stagedAssetID], successor.context.key)
        XCTAssertEqual(snapshot.reservedBytes, replacement.reservedBytes)
        XCTAssertEqual(snapshot.feedback[feedback.id]?.phase, .acknowledged)
        XCTAssertNil(snapshot.operations[old.context.key]?.stagedAssetID)
        XCTAssertEqual(snapshot.operations[old.context.key]?.reservedBytes, 0)
    }

    func testWatermarksAreMonotonicAndPersistAcrossFakeRelaunch() async throws {
        let fake = GaryxFakeComposerDurabilityStore()
        let committed = try await fake.commit(
            .init(
                expectedRevision: 0,
                label: "reserve hi-lo blocks",
                mutations: [
                    .setGenerationHighWatermark(64),
                    .setReservationHighWatermark(32),
                ]
            )
        )
        let encoded = try JSONEncoder().encode(committed)
        let decoded = try JSONDecoder().decode(
            GaryxComposerDurabilitySnapshot.self,
            from: encoded
        )
        let relaunched = GaryxFakeComposerDurabilityStore(initial: decoded)
        let restored = try await relaunched.load()
        XCTAssertEqual(restored, committed)

        do {
            _ = try await relaunched.commit(
                .init(
                    expectedRevision: committed.revision,
                    label: "regression",
                    mutations: [.setGenerationHighWatermark(63)]
                )
            )
            XCTFail("watermark regression must fail")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected error \(error)")
            }
        }
        let unchanged = try await relaunched.load()
        XCTAssertEqual(unchanged, committed)
    }

    func testDiscardConvergenceResumesAcrossSnapshotRelaunchAtEveryBoundary() async throws {
        var convergence = makeDiscardConvergence()
        var store = GaryxFakeComposerDurabilityStore()
        var snapshot = try await store.commit(
            .init(
                expectedRevision: 0,
                label: "enter discarding",
                mutations: [.upsertDiscardConvergence(convergence)]
            )
        )

        for step in 0..<4 {
            convergence = try XCTUnwrap(snapshot.discardConvergence[entryID])
            switch step {
            case 0: convergence.settleDeliveries()
            case 1: convergence.settleReservation()
            case 2: convergence.settleSessions()
            case 3: convergence.settleResources()
            default: break
            }
            snapshot = try await store.commit(
                .init(
                    expectedRevision: snapshot.revision,
                    label: "discard step \(step)",
                    mutations: [.upsertDiscardConvergence(convergence)]
                )
            )
            let encoded = try JSONEncoder().encode(snapshot)
            let decoded = try JSONDecoder().decode(
                GaryxComposerDurabilitySnapshot.self,
                from: encoded
            )
            store = GaryxFakeComposerDurabilityStore(initial: decoded)
            snapshot = try await store.load()
        }

        convergence = try XCTUnwrap(snapshot.discardConvergence[entryID])
        XCTAssertTrue(convergence.finishToken())
        snapshot = try await store.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "token discarded",
                mutations: [.upsertDiscardConvergence(convergence)]
            )
        )
        XCTAssertEqual(snapshot.discardConvergence[entryID]?.lifecycle.phase, .discarded)
        XCTAssertTrue(snapshot.discardConvergence[entryID]?.descendantsEmpty == true)
        XCTAssertTrue(snapshot.discardConvergence[entryID]?.deliveriesSettled == true)
    }

    func testCorrelationAndDiscardTombstonesSharePersistentCountAndByteBudget() async throws {
        var convergence = makeDiscardConvergence()
        convergence.settleReservation()
        convergence.settleDeliveries()
        convergence.settleSessions()
        convergence.settleResources()
        XCTAssertTrue(convergence.finishToken())
        let convergenceRoundTrip = try JSONDecoder().decode(
            GaryxPayloadDiscardConvergence.self,
            from: JSONEncoder().encode(convergence)
        )
        XCTAssertEqual(convergenceRoundTrip, convergence)
        XCTAssertEqual(convergenceRoundTrip.persistentTombstoneCount, 1)
        var delivery = makeDelivery()
        XCTAssertTrue(delivery.markTransportAttempted())
        delivery.recordServerAcknowledgement()
        let nonTerminal = GaryxDeliveryRecord(
            id: .init(rawValue: "aa-non-terminal-delivery"),
            scope: scope,
            entryID: entryID,
            reservationID: reservation,
            correlationID: "non-terminal-correlation",
            envelope: makeEnvelope(text: "still pending")
        )
        let ledger = makeLedger(outcome: .committed)
        let expectedBytes = try XCTUnwrap(delivery.persistentTombstoneEstimatedBytes)
            + convergence.persistentTombstoneBytes

        let compacting = GaryxComposerDurabilitySnapshot(
            ledgers: [ledger.key: ledger],
            tombstoneBudget: .init(countLimit: 1, byteLimit: expectedBytes)
        )
        let compactedStore = GaryxFakeComposerDurabilityStore(initial: compacting)
        let compacted = try await compactedStore.commit(
            .init(
                expectedRevision: 0,
                label: "compact correlation for finalization tombstone",
                mutations: [
                    .upsertDelivery(delivery),
                    .upsertDelivery(nonTerminal),
                    .upsertDiscardConvergence(convergence),
                ]
            )
        )
        XCTAssertNil(compacted.deliveries[delivery.id])
        XCTAssertEqual(compacted.deliveries[nonTerminal.id], nonTerminal)
        XCTAssertEqual(compacted.discardConvergence[entryID], convergence)
        XCTAssertEqual(
            compacted.persistentTombstoneUsage,
            .init(
                discardFinalizationCount: 1,
                discardFinalizationBytes: convergence.persistentTombstoneBytes
            )
        )

        let byteCompacting = GaryxComposerDurabilitySnapshot(
            ledgers: [ledger.key: ledger],
            tombstoneBudget: .init(
                countLimit: 2,
                byteLimit: convergence.persistentTombstoneBytes
            )
        )
        let byteCompactedStore = GaryxFakeComposerDurabilityStore(initial: byteCompacting)
        let byteCompacted = try await byteCompactedStore.commit(
            .init(
                expectedRevision: 0,
                label: "compact correlation for tombstone byte budget",
                mutations: [
                    .upsertDelivery(delivery),
                    .upsertDiscardConvergence(convergence),
                ]
            )
        )
        XCTAssertNil(byteCompacted.deliveries[delivery.id])
        XCTAssertEqual(byteCompacted.discardConvergence[entryID], convergence)
        XCTAssertEqual(
            byteCompacted.persistentTombstoneUsage,
            .init(
                discardFinalizationCount: 1,
                discardFinalizationBytes: convergence.persistentTombstoneBytes
            )
        )

        let finalizationOnlyOverBudget = GaryxComposerDurabilitySnapshot(
            ledgers: [ledger.key: ledger],
            tombstoneBudget: .init(countLimit: 0, byteLimit: expectedBytes)
        )
        let rejected = GaryxFakeComposerDurabilityStore(initial: finalizationOnlyOverBudget)
        do {
            _ = try await rejected.commit(
                .init(
                    expectedRevision: 0,
                    label: "unprunable finalization tombstone over budget",
                    mutations: [.upsertDiscardConvergence(convergence)]
                )
            )
            XCTFail("discard finalization tombstones must never be silently evicted")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected error \(error)")
            }
        }
        let unchanged = try await rejected.load()
        XCTAssertEqual(unchanged, finalizationOnlyOverBudget)

        let admitted = GaryxFakeComposerDurabilityStore(
            initial: GaryxComposerDurabilitySnapshot(
                ledgers: [ledger.key: ledger],
                tombstoneBudget: .init(countLimit: 2, byteLimit: expectedBytes)
            )
        )
        var snapshot = try await admitted.commit(
            .init(
                expectedRevision: 0,
                label: "fill tombstone pool",
                mutations: [
                    .upsertDelivery(delivery),
                    .upsertDiscardConvergence(convergence),
                ]
            )
        )
        XCTAssertEqual(
            snapshot.persistentTombstoneUsage,
            GaryxPersistentTombstoneUsage(
                correlationCount: 1,
                correlationBytes: try XCTUnwrap(delivery.persistentTombstoneEstimatedBytes),
                discardFinalizationCount: 1,
                discardFinalizationBytes: convergence.persistentTombstoneBytes
            )
        )

        let removalRoundTrip = try JSONDecoder().decode(
            GaryxComposerDurabilitySnapshot.self,
            from: JSONEncoder().encode(snapshot)
        )
        XCTAssertEqual(removalRoundTrip, snapshot)
        let relaunchedForRemoval = GaryxFakeComposerDurabilityStore(initial: removalRoundTrip)
        snapshot = try await relaunchedForRemoval.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "reclaim tombstone pool",
                mutations: [
                    .removeDelivery(delivery.id),
                    .removeDiscardConvergence(entryID),
                ]
            )
        )
        XCTAssertEqual(snapshot.persistentTombstoneUsage, .init())
    }

    func testDiscardConvergenceCannotBeGarbageCollectedBeforeDiscarded() async throws {
        let convergence = makeDiscardConvergence()
        let initial = GaryxComposerDurabilitySnapshot(
            discardConvergence: [entryID: convergence]
        )
        let fake = GaryxFakeComposerDurabilityStore(initial: initial)

        do {
            _ = try await fake.commit(
                .init(
                    expectedRevision: 0,
                    label: "premature discard GC",
                    mutations: [.removeDiscardConvergence(entryID)]
                )
            )
            XCTFail("discarding convergence must remain durable")
        } catch {
            XCTAssertEqual(
                error as? GaryxComposerDurabilityError,
                .invariantViolation("discard convergence GC requires discarded lifecycle")
            )
        }
        let unchanged = try await fake.load()
        XCTAssertEqual(unchanged, initial)
    }

    func testTerminalLedgerWithoutDurableDescendantsIsRetiredOnNextTransaction() async throws {
        let ledger = makeLedger(outcome: .revoked)
        let initial = GaryxComposerDurabilitySnapshot(ledgers: [ledger.key: ledger])
        let fake = GaryxFakeComposerDurabilityStore(initial: initial)

        let compacted = try await fake.commit(
            .init(expectedRevision: 0, label: "compact descendant-free ledger", mutations: [])
        )

        XCTAssertTrue(compacted.ledgers.isEmpty)
    }

    func testTerminalLedgerRetiresOnlyAfterItsLastDurableDescendant() async throws {
        let ledger = makeLedger(outcome: .committed)
        let delivery = makeDelivery()
        let fake = GaryxFakeComposerDurabilityStore()
        var snapshot = try await fake.commit(
            .init(
                expectedRevision: 0,
                label: "root terminal ledger by delivery",
                mutations: [.upsertLedger(ledger), .upsertDelivery(delivery)]
            )
        )
        XCTAssertEqual(snapshot.ledgers[ledger.key], ledger)

        do {
            _ = try await fake.commit(
                .init(
                    expectedRevision: snapshot.revision,
                    label: "reject rooted ledger GC",
                    mutations: [.removeLedger(ledger.key)]
                )
            )
            XCTFail("a ledger with a durable descendant must not be removed")
        } catch {
            XCTAssertEqual(
                error as? GaryxComposerDurabilityError,
                .invariantViolation(
                    "reservation ledger GC requires terminal descendant-free state"
                )
            )
        }

        snapshot = try await fake.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "remove final reservation descendant",
                mutations: [.removeDelivery(delivery.id)]
            )
        )
        XCTAssertNil(snapshot.deliveries[delivery.id])
        XCTAssertNil(snapshot.ledgers[ledger.key])
    }

    func testTerminalLedgerRemainsWhileOnlyDiscardConvergenceCapturesReservation() async throws {
        let ledger = makeLedger(outcome: .revoked)
        var convergence = makeDiscardConvergence()
        convergence.settleReservation()
        convergence.settleDeliveries()
        convergence.settleSessions()
        convergence.settleResources()
        XCTAssertTrue(convergence.finishToken())
        let fake = GaryxFakeComposerDurabilityStore()

        var snapshot = try await fake.commit(
            .init(
                expectedRevision: 0,
                label: "retain terminal ledger through convergence record GC window",
                mutations: [
                    .upsertLedger(ledger),
                    .upsertDiscardConvergence(convergence),
                ]
            )
        )
        XCTAssertEqual(snapshot.ledgers[ledger.key], ledger)
        XCTAssertEqual(snapshot.discardConvergence[entryID], convergence)

        snapshot = try await fake.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "retire terminal ledger with final convergence descendant",
                mutations: [.removeDiscardConvergence(entryID)]
            )
        )
        XCTAssertNil(snapshot.discardConvergence[entryID])
        XCTAssertNil(snapshot.ledgers[ledger.key])
    }

    func testUnsettledReservationLedgerBudgetsAreFailClosed() async throws {
        func ledger(index: Int, scope: GaryxGatewayScope) -> GaryxProvisionalReservationLedger {
            GaryxProvisionalReservationLedger(
                key: .init(
                    scope: scope,
                    entryID: .init(rawValue: "entry-\(index)"),
                    reservationID: .init(rawValue: UInt64(index + 1))
                ),
                envelopeGeneration: 10,
                followupGeneration: 11
            )
        }
        let fake = GaryxFakeComposerDurabilityStore()
        let overGlobalLimit = (0...GaryxProvisionalReservationLedger.unsettledGlobalLimit)
            .map { index in
                GaryxComposerDurabilityMutation.upsertLedger(
                    ledger(
                        index: index,
                        scope: .init(identity: "scope-\(index % 5)", epoch: 1)
                    )
                )
            }
        do {
            _ = try await fake.commit(
                .init(
                    expectedRevision: 0,
                    label: "reject global unsettled-ledger overflow",
                    mutations: overGlobalLimit
                )
            )
            XCTFail("unsettled ledgers must have a global bound")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected error \(error)")
            }
        }
        let unchanged = try await fake.load()
        XCTAssertTrue(unchanged.ledgers.isEmpty)

        let perScope = GaryxFakeComposerDurabilityStore()
        let overPerScopeLimit = (0...GaryxProvisionalReservationLedger.unsettledPerScopeLimit)
            .map { index in
                GaryxComposerDurabilityMutation.upsertLedger(
                    ledger(index: index, scope: scope)
                )
            }
        do {
            _ = try await perScope.commit(
                .init(
                    expectedRevision: 0,
                    label: "reject per-scope unsettled-ledger overflow",
                    mutations: overPerScopeLimit
                )
            )
            XCTFail("unsettled ledgers must have a per-scope bound")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected error \(error)")
            }
        }

        let byteBounded = GaryxFakeComposerDurabilityStore()
        let oversizedScope = GaryxGatewayScope(
            identity: String(
                repeating: "x",
                count: GaryxProvisionalReservationLedger.unsettledByteLimit
            ),
            epoch: 1
        )
        do {
            _ = try await byteBounded.commit(
                .init(
                    expectedRevision: 0,
                    label: "reject unsettled-ledger byte overflow",
                    mutations: [.upsertLedger(ledger(index: 0, scope: oversizedScope))]
                )
            )
            XCTFail("unsettled ledgers must have a byte bound")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected error \(error)")
            }
        }
    }

    func testBarrierRemovalRequiresPayloadFreeIdleState() async throws {
        let idle = makeBarrier()
        let removable = GaryxFakeComposerDurabilityStore(
            initial: .init(barriers: [entryID: idle])
        )
        let removed = try await removable.commit(
            .init(
                expectedRevision: 0,
                label: "remove payload-free idle barrier",
                mutations: [.removeBarrier(entryID)]
            )
        )
        XCTAssertTrue(removed.barriers.isEmpty)

        let ledger = makeLedger(outcome: nil)
        var sealed = makeBarrier()
        XCTAssertEqual(
            sealed.seal(
                reservationID: reservation,
                envelope: makeEnvelope(text: "sealed"),
                followupGeneration: 11,
                readiness: .ready,
                quota: .init(),
                producerPhase: .live,
                lifecycle: makeEntry().lifecycle.snapshot
            ),
            .sealed
        )
        let protected = GaryxFakeComposerDurabilityStore(
            initial: .init(barriers: [entryID: sealed], ledgers: [ledger.key: ledger])
        )
        do {
            _ = try await protected.commit(
                .init(
                    expectedRevision: 0,
                    label: "reject live barrier GC",
                    mutations: [.removeBarrier(entryID)]
                )
            )
            XCTFail("a sealed barrier must not be removed")
        } catch {
            XCTAssertEqual(
                error as? GaryxComposerDurabilityError,
                .invariantViolation("send barrier GC requires idle phase")
            )
        }
    }

    func testClaimedGenerationHistoryFailsClosedAtItsBound() async throws {
        let initial = GaryxComposerDurabilitySnapshot(generationHighWatermark: 8_192)
        let fake = GaryxFakeComposerDurabilityStore(initial: initial)
        let mutations = (1...4_097).map {
            GaryxComposerDurabilityMutation.claimGeneration(UInt64($0))
        }

        do {
            _ = try await fake.commit(
                .init(
                    expectedRevision: 0,
                    label: "reject unbounded claimed-generation history",
                    mutations: mutations
                )
            )
            XCTFail("claimed-generation replay history must remain bounded")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected error \(error)")
            }
        }
        let unchanged = try await fake.load()
        XCTAssertEqual(unchanged, initial)
    }

    func testAdvancingHiLoBlockCompactsExactClaimsBehindPermanentFloor() async throws {
        let initial = GaryxComposerDurabilitySnapshot(
            generationHighWatermark: 32,
            claimedGenerations: [12, 31]
        )
        let fake = GaryxFakeComposerDurabilityStore(initial: initial)
        var snapshot = try await fake.commit(
            .init(
                expectedRevision: 0,
                label: "advance generation block and retire old exact claims",
                mutations: [.setGenerationHighWatermark(64)]
            )
        )
        XCTAssertEqual(snapshot.generationClaimFloor, 32)
        XCTAssertTrue(snapshot.claimedGenerations.isEmpty)

        do {
            _ = try await fake.commit(
                .init(
                    expectedRevision: snapshot.revision,
                    label: "reject a claim behind the permanent floor",
                    mutations: [.claimGeneration(12)]
                )
            )
            XCTFail("a compacted generation must remain permanently unclaimable")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected error \(error)")
            }
        }

        snapshot = try await fake.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "claim within current exact replay window",
                mutations: [.claimGeneration(33)]
            )
        )
        XCTAssertEqual(snapshot.claimedGenerations, [33])
        let roundTrip = try JSONDecoder().decode(
            GaryxComposerDurabilitySnapshot.self,
            from: JSONEncoder().encode(snapshot)
        )
        XCTAssertEqual(roundTrip.generationClaimFloor, 32)
        XCTAssertEqual(roundTrip.claimedGenerations, [33])
    }

    func testTerminalCreateDeliveryHistorySharesThePersistentTombstoneBudget() async throws {
        func acknowledgedCreate(_ intentID: String) -> GaryxCreateDeliveryState {
            var create = GaryxCreateDeliveryState(scope: scope, createIntentID: intentID)
            create.created(threadID: "thread-\(intentID)")
            create.chatStartAttempted()
            create.acknowledged()
            return create
        }
        let first = acknowledgedCreate("create-0001")
        let second = acknowledgedCreate("create-0002")
        var nonTerminal = GaryxCreateDeliveryState(
            scope: scope,
            createIntentID: "aa-ambiguous-non-terminal-create"
        )
        nonTerminal.created(threadID: "ambiguous-thread")
        nonTerminal.chatStartAttempted()
        nonTerminal.responseLost()
        XCTAssertEqual(nonTerminal.phase, .ambiguous)
        XCTAssertEqual(nonTerminal.userDisposition, .none)
        let fake = GaryxFakeComposerDurabilityStore(
            initial: .init(tombstoneBudget: .init(countLimit: 1, byteLimit: 4 * 1024 * 1024))
        )

        let compacted = try await fake.commit(
            .init(
                expectedRevision: 0,
                label: "bound terminal create-delivery history",
                mutations: [
                    .upsertCreateDelivery(nonTerminal),
                    .upsertCreateDelivery(first),
                    .upsertCreateDelivery(second),
                ]
            )
        )

        XCTAssertEqual(compacted.createDeliveries[nonTerminal.key], nonTerminal)
        XCTAssertNil(compacted.createDeliveries[first.key])
        XCTAssertEqual(compacted.createDeliveries[second.key], second)
        XCTAssertEqual(compacted.persistentTombstoneUsage.count, 1)

        let byteBounded = GaryxFakeComposerDurabilityStore(
            initial: .init(
                tombstoneBudget: .init(countLimit: 2, byteLimit: second.estimatedBytes)
            )
        )
        let byteCompacted = try await byteBounded.commit(
            .init(
                expectedRevision: 0,
                label: "bound terminal create-delivery bytes",
                mutations: [.upsertCreateDelivery(first), .upsertCreateDelivery(second)]
            )
        )
        XCTAssertNil(byteCompacted.createDeliveries[first.key])
        XCTAssertEqual(byteCompacted.createDeliveries[second.key], second)
        XCTAssertEqual(byteCompacted.persistentTombstoneUsage.bytes, second.estimatedBytes)
    }

    func testCorrelationCompactionRetiresCreateBeforeDelivery() async throws {
        var create = GaryxCreateDeliveryState(scope: scope, createIntentID: "terminal-create")
        create.created(threadID: "created-thread")
        create.chatStartAttempted()
        create.acknowledged()
        var delivery = makeDelivery()
        XCTAssertTrue(delivery.markTransportAttempted())
        delivery.recordServerAcknowledgement()
        let ledger = makeLedger(outcome: .committed)
        let fake = GaryxFakeComposerDurabilityStore(
            initial: .init(tombstoneBudget: .init(countLimit: 1, byteLimit: 4 * 1024 * 1024))
        )

        let compacted = try await fake.commit(
            .init(
                expectedRevision: 0,
                label: "retire create correlation before delivery correlation",
                mutations: [
                    .upsertLedger(ledger),
                    .upsertDelivery(delivery),
                    .upsertCreateDelivery(create),
                ]
            )
        )

        XCTAssertNil(compacted.createDeliveries[create.key])
        XCTAssertEqual(compacted.deliveries[delivery.id], delivery)
        XCTAssertEqual(compacted.ledgers[ledger.key], ledger)
        XCTAssertEqual(compacted.persistentTombstoneUsage.correlationCount, 1)
        XCTAssertEqual(compacted.persistentTombstoneUsage.createCorrelationCount, 0)
    }

    func testCreateDeliveryRemovalRequiresTerminalCorrelation() async throws {
        var ambiguous = GaryxCreateDeliveryState(
            scope: scope,
            createIntentID: "ambiguous-without-disposition"
        )
        ambiguous.created(threadID: "ambiguous-thread")
        ambiguous.chatStartAttempted()
        ambiguous.responseLost()
        var acknowledged = GaryxCreateDeliveryState(
            scope: scope,
            createIntentID: "acknowledged-create"
        )
        acknowledged.created(threadID: "acknowledged-thread")
        acknowledged.chatStartAttempted()
        acknowledged.acknowledged()
        let fake = GaryxFakeComposerDurabilityStore()
        var snapshot = try await fake.commit(
            .init(
                expectedRevision: 0,
                label: "seed terminal and unresolved create correlations",
                mutations: [
                    .upsertCreateDelivery(ambiguous),
                    .upsertCreateDelivery(acknowledged),
                ]
            )
        )

        do {
            _ = try await fake.commit(
                .init(
                    expectedRevision: snapshot.revision,
                    label: "reject unresolved create correlation GC",
                    mutations: [.removeCreateDelivery(ambiguous.key)]
                )
            )
            XCTFail("ambiguous create without user disposition is non-terminal")
            return
        } catch {
            XCTAssertEqual(
                error as? GaryxComposerDurabilityError,
                .invariantViolation("create delivery GC requires terminal correlation")
            )
        }
        let unchanged = try await fake.load()
        XCTAssertEqual(unchanged, snapshot)

        snapshot = try await fake.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "remove terminal create correlation",
                mutations: [.removeCreateDelivery(acknowledged.key)]
            )
        )
        XCTAssertEqual(snapshot.createDeliveries[ambiguous.key], ambiguous)
        XCTAssertNil(snapshot.createDeliveries[acknowledged.key])
    }

    func testNonTerminalCreateDeliveryBudgetIsFailClosed() async throws {
        let fake = GaryxFakeComposerDurabilityStore()
        var mutations = (0..<GaryxCreateDeliveryState.nonTerminalGlobalLimit).map { index in
            GaryxComposerDurabilityMutation.upsertCreateDelivery(
                .init(
                    scope: .init(identity: "create-scope-\(index % 5)", epoch: 1),
                    createIntentID: "create-\(index)"
                )
            )
        }
        var unresolvedAmbiguous = GaryxCreateDeliveryState(
            scope: .init(identity: "ambiguous-create-scope", epoch: 1),
            createIntentID: "ambiguous-without-disposition"
        )
        unresolvedAmbiguous.created(threadID: "ambiguous-thread")
        unresolvedAmbiguous.chatStartAttempted()
        unresolvedAmbiguous.responseLost()
        mutations.append(.upsertCreateDelivery(unresolvedAmbiguous))
        do {
            _ = try await fake.commit(
                .init(
                    expectedRevision: 0,
                    label: "reject non-terminal create overflow",
                    mutations: mutations
                )
            )
            XCTFail("non-terminal create delivery state must remain bounded")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected error \(error)")
            }
        }
        let unchanged = try await fake.load()
        XCTAssertTrue(unchanged.createDeliveries.isEmpty)

        let perScope = GaryxFakeComposerDurabilityStore()
        let perScopeMutations = (0...GaryxCreateDeliveryState.nonTerminalPerScopeLimit)
            .map { index in
                GaryxComposerDurabilityMutation.upsertCreateDelivery(
                    .init(scope: scope, createIntentID: "per-scope-create-\(index)")
                )
            }
        do {
            _ = try await perScope.commit(
                .init(
                    expectedRevision: 0,
                    label: "reject per-scope create overflow",
                    mutations: perScopeMutations
                )
            )
            XCTFail("non-terminal create state must have a per-scope bound")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected error \(error)")
            }
        }

        let byteBounded = GaryxFakeComposerDurabilityStore()
        let oversized = GaryxCreateDeliveryState(
            scope: scope,
            createIntentID: String(
                repeating: "c",
                count: GaryxCreateDeliveryState.nonTerminalByteLimit
            )
        )
        do {
            _ = try await byteBounded.commit(
                .init(
                    expectedRevision: 0,
                    label: "reject non-terminal create byte overflow",
                    mutations: [.upsertCreateDelivery(oversized)]
                )
            )
            XCTFail("non-terminal create state must have a byte bound")
        } catch let error as GaryxComposerDurabilityError {
            guard case .invariantViolation = error else {
                return XCTFail("unexpected error \(error)")
            }
        }
    }

    func testScopePartitionedComposerSurvivesSnapshotRoundTrip() async throws {
        let otherScope = GaryxGatewayScope(identity: "other", epoch: 1)
        var payloadStore = GaryxComposerPayloadStore()
        let first = makeEntry(text: "G1")
        let otherID = GaryxComposerPayloadEntryID(rawValue: "other-entry")
        let second = GaryxComposerPayloadEntry(
            id: otherID,
            scope: otherScope,
            destination: .draft("other"),
            lifecycleToken: GaryxPayloadLifecycleToken(entryID: otherID, nonce: "other-token"),
            currentGeneration: 1,
            text: "G2"
        )
        XCTAssertTrue(payloadStore.insert(first))
        XCTAssertTrue(payloadStore.insert(second))
        let snapshot = GaryxComposerDurabilitySnapshot(payloadStore: payloadStore)
        let encoded = try JSONEncoder().encode(snapshot)
        let restored = try JSONDecoder().decode(GaryxComposerDurabilitySnapshot.self, from: encoded)
        XCTAssertEqual(restored.payloadStore.entry(first.id, scope: scope)?.currentText, "G1")
        XCTAssertEqual(restored.payloadStore.entry(second.id, scope: otherScope)?.currentText, "G2")
    }

    private struct CommitProbe: Sendable {
        let isSuccess: Bool
    }

    private func commitResult(
        store: GaryxFakeComposerDurabilityStore,
        transaction: GaryxComposerDurabilityTransaction
    ) async -> CommitProbe {
        do {
            _ = try await store.commit(transaction)
            return CommitProbe(isSuccess: true)
        } catch {
            return CommitProbe(isSuccess: false)
        }
    }

    private var entryID: GaryxComposerPayloadEntryID {
        GaryxComposerPayloadEntryID(rawValue: "entry")
    }

    private var reservation: GaryxSendReservationID {
        GaryxSendReservationID(rawValue: 1)
    }

    private var deliveryID: GaryxDeliveryRecordID {
        GaryxDeliveryRecordID(rawValue: "delivery")
    }

    private func makeEntry(id: String = "entry", text: String = "") -> GaryxComposerPayloadEntry {
        let entryID = GaryxComposerPayloadEntryID(rawValue: id)
        return GaryxComposerPayloadEntry(
            id: entryID,
            scope: scope,
            destination: .draft("D"),
            lifecycleToken: GaryxPayloadLifecycleToken(entryID: entryID, nonce: "token-\(id)"),
            currentGeneration: 10,
            text: text
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

    private func makeBarrier() -> GaryxSendCommitBarrier {
        let entry = makeEntry()
        return GaryxSendCommitBarrier(
            entryID: entry.id,
            scope: scope,
            payloadLifecycle: GaryxPayloadLifecycleCapture(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
        )
    }

    private func makeLedger(
        outcome: GaryxReservationTerminalOutcome?
    ) -> GaryxProvisionalReservationLedger {
        var ledger = GaryxProvisionalReservationLedger(
            key: GaryxReservationLedgerKey(
                scope: scope,
                entryID: entryID,
                reservationID: reservation
            ),
            envelopeGeneration: 10,
            followupGeneration: 11
        )
        if let outcome {
            ledger.settle(outcome, targetGeneration: outcome == .committed ? 11 : 12)
        }
        return ledger
    }

    private func operationKey(
        _ id: String = "operation",
        reservationID: GaryxSendReservationID? = nil
    ) -> GaryxOperationCapabilityKey {
        GaryxOperationCapabilityKey(
            scope: scope,
            entryID: entryID,
            generation: 11,
            reservationID: reservationID,
            branch: .followup,
            operationID: GaryxOperationID(rawValue: id)
        )
    }

    private func makeOperation(
        id: String = "operation",
        state: GaryxOperationCapabilityState,
        assetID: GaryxStagedAssetID? = nil,
        reservedBytes: Int = 0
    ) -> GaryxOperationCapability {
        let entry = makeEntry()
        let key = operationKey(id)
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
            state: state,
            stagedAssetID: assetID,
            reservedBytes: reservedBytes
        )
    }

    private func makeManifest(
        reservationID: GaryxSendReservationID?
    ) -> GaryxOperationManifest {
        GaryxOperationManifest(
            key: operationKey(reservationID: reservationID),
            stagedPath: "staging/asset.bin",
            state: .preparing,
            uploadAttempted: false
        )
    }

    private func makeReplacement(
        oldKey: GaryxOperationCapabilityKey? = nil,
        reservationID: GaryxSendReservationID?
    ) -> GaryxReplacementRecord {
        GaryxReplacementRecord(
            id: GaryxReplacementID(rawValue: "replacement"),
            scope: scope,
            entryID: entryID,
            oldKey: oldKey ?? operationKey("old", reservationID: reservationID),
            reservationID: reservationID,
            branch: .followup,
            stagedAssetID: GaryxStagedAssetID(rawValue: "replacement-asset"),
            reservedBytes: 100
        )
    }

    private func makeFeedback() -> GaryxOperationFeedback {
        GaryxOperationFeedback(
            id: GaryxFeedbackID(rawValue: "feedback"),
            scope: scope,
            entryID: entryID,
            operationID: GaryxOperationID(rawValue: "operation"),
            kind: .uploadTerminal
        )
    }

    private func makeDurableProducerDrained() -> (
        key: GaryxSessionDescendantKey,
        value: GaryxDurableProducerDrainedRecord
    ) {
        makeDurableProducerDrained(reservationID: reservation)
    }

    private func makeDurableProducerDrained(
        reservationID: GaryxSendReservationID?
    ) -> (
        key: GaryxSessionDescendantKey,
        value: GaryxDurableProducerDrainedRecord
    ) {
        let entry = makeEntry()
        let key = GaryxSessionDescendantKey(
            token: entry.lifecycle.token,
            sessionID: GaryxComposerInputSessionID(rawValue: "session"),
            epoch: 1
        )
        return (
            key,
            GaryxDurableProducerDrainedRecord(
                scope: scope,
                entryID: entryID,
                reservationID: reservationID,
                record: GaryxProducerDrainedRecord(
                    sessionID: key.sessionID,
                    epoch: key.epoch,
                    finalSequence: 3,
                    bufferedText: "U"
                )
            )
        )
    }

    private func makeDelivery() -> GaryxDeliveryRecord {
        GaryxDeliveryRecord(
            id: deliveryID,
            scope: scope,
            entryID: entryID,
            reservationID: reservation,
            correlationID: "correlation",
            envelope: makeEnvelope(text: "T")
        )
    }

    private func makeDiscardConvergence() -> GaryxPayloadDiscardConvergence {
        var entry = makeEntry(text: "payload")
        XCTAssertTrue(entry.beginDiscard(revision: 9))
        var barrier = makeBarrier()
        let active = GaryxPayloadLifecycleSnapshot(
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
            lifecycle: active
        )
        let sessionKey = GaryxSessionDescendantKey(
            token: entry.lifecycle.token,
            sessionID: GaryxComposerInputSessionID(rawValue: "session"),
            epoch: 1
        )
        var delivery = makeDelivery()
        _ = delivery.markTransportAttempted()
        _ = delivery.markAmbiguous()
        let operation = makeOperation(
            state: .failedRetryable,
            assetID: GaryxStagedAssetID(rawValue: "asset"),
            reservedBytes: 100
        )
        return GaryxPayloadDiscardConvergence(
            lifecycle: entry.lifecycle,
            barrier: barrier,
            sessions: [
                sessionKey: GaryxSessionDescendant(
                    key: sessionKey,
                    composerKey: .draft("D"),
                    phase: .closePendingAck,
                    finalSequence: 3
                ),
            ],
            deliveries: [delivery.id: delivery],
            operations: [operation.context.key: operation],
            stagedAssetIDs: [GaryxStagedAssetID(rawValue: "asset")],
            reservedBytes: 100
        )
    }

    private func mutateDeliveryJSON(
        _ record: GaryxDeliveryRecord,
        mutation: (inout [String: Any]) throws -> Void
    ) throws -> GaryxDeliveryRecord {
        var object = try encodedJSONObject(record)
        try mutation(&object)
        let data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return try JSONDecoder().decode(GaryxDeliveryRecord.self, from: data)
    }

    private func encodedJSONObject<T: Encodable>(_ value: T) throws -> [String: Any] {
        let encoded = try JSONEncoder().encode(value)
        return try XCTUnwrap(
            JSONSerialization.jsonObject(with: encoded) as? [String: Any]
        )
    }

    private func encodedJSONValue<T: Encodable>(_ value: T) throws -> Any {
        let encoded = try JSONEncoder().encode(value)
        return try JSONSerialization.jsonObject(with: encoded, options: [.fragmentsAllowed])
    }
}
