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
        let replacement = makeReplacement(reservationID: reservation)
        let drained = makeDurableProducerDrained(reservationID: reservation)

        for descendant in [
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
        let committed = try await fake.commit(
            .init(
                expectedRevision: 0,
                label: "correct admission order",
                mutations: [
                    .upsertLedger(ledger),
                    .upsertManifest(manifest),
                    .upsertReplacement(replacement),
                    .upsertProducerDrained(drained.key, drained.value),
                ]
            )
        )
        XCTAssertEqual(committed.ledgers.count, 1)
        XCTAssertEqual(committed.manifests.count, 1)
        XCTAssertEqual(committed.replacements.count, 1)
        XCTAssertEqual(committed.producerDrained.count, 1)
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
        var followupEntry = entry
        followupEntry.setText(settlement.followupText, generation: settlement.followupGeneration)
        let delivery = try XCTUnwrap(settlement.deliveryRecord)

        let fake = GaryxFakeComposerDurabilityStore()
        for failpoint in 0..<4 {
            let isolated = GaryxFakeComposerDurabilityStore()
            await isolated.injectFailure(atMutationIndex: failpoint)
            do {
                _ = try await isolated.commit(
                    .init(
                        expectedRevision: 0,
                        label: "commitSend failpoint",
                        mutations: [
                            .upsertLedger(ledger),
                            .upsertEntry(followupEntry),
                            .upsertBarrier(barrier),
                            .upsertDelivery(delivery),
                        ]
                    )
                )
                XCTFail("expected failure")
            } catch {}
            let unchanged = try await isolated.load()
            XCTAssertEqual(unchanged, GaryxComposerDurabilitySnapshot())
        }

        let snapshot = try await fake.commit(
            .init(
                expectedRevision: 0,
                label: "commitSend",
                mutations: [
                    .upsertLedger(ledger),
                    .upsertEntry(followupEntry),
                    .upsertBarrier(barrier),
                    .upsertDelivery(delivery),
                ]
            )
        )
        XCTAssertEqual(snapshot.ledgers[ledger.key]?.terminalOutcome, .committed)
        XCTAssertEqual(snapshot.payloadStore.entry(entry.id, scope: scope)?.currentText, "U")
        XCTAssertEqual(snapshot.barriers[entry.id]?.phase, .durableCommitted)
        XCTAssertEqual(snapshot.deliveries[delivery.id], delivery)
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

    func testFailedTerminalFeedbackEntryReferenceAndOperationAreAtomic() async throws {
        var entry = makeEntry()
        let feedback = makeFeedback()
        entry.addFeedbackReference(feedback.id)
        let operation = makeOperation(state: .failedTerminal)
        let mutations: [GaryxComposerDurabilityMutation] = [
            .upsertOperation(operation),
            .upsertFeedback(feedback),
            .upsertEntry(entry),
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
        let fresh = makeOperation(id: "fresh", state: .requested)
        XCTAssertTrue(
            retainedLineage.admitsFreshOperation(
                fresh,
                feedback: pendingFeedback,
                lifecycle: entry.lifecycle.snapshot
            )
        )

        var acknowledgedFeedback = pendingFeedback
        acknowledgedFeedback.acknowledge()
        var releasedLineage = retainedLineage
        XCTAssertTrue(releasedLineage.release(after: acknowledgedFeedback))

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
        let entry = makeEntry()
        let scopes = GaryxGatewayScopeRegistry(initialActiveScope: scope)
        XCTAssertEqual(
            GaryxReplacementSwapReducer.commit(
                old: &old,
                successor: &successor,
                record: &replacement,
                lifecycle: entry.lifecycle.snapshot,
                scopes: scopes
            ),
            .committed
        )
        let mutations: [GaryxComposerDurabilityMutation] = [
            .upsertOperation(old),
            .upsertOperation(successor),
            .upsertReplacement(replacement),
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

    private func makeEnvelope(text: String) -> GaryxDeliveryEnvelope {
        GaryxDeliveryEnvelope(
            text: text,
            attachmentIDs: [],
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
}
