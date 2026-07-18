import XCTest
@testable import GaryxMobileCore

final class GaryxComposerPayloadLifecycleTests: XCTestCase {
    private let scope = GaryxGatewayScope(identity: "gateway", epoch: 1)

    func testPayloadEntryChildIdentityKeepsTextAndSiblingWhenOneOperationFails() {
        var entry = makeEntry(text: "draft")
        let first = attachment("first", generation: 1)
        let second = attachment("second", generation: 1)
        entry.addAttachment(first)
        entry.addAttachment(second)
        let firstOperation = operationKey("op-first")
        let secondOperation = operationKey("op-second")
        entry.addOperation(firstOperation)
        entry.addOperation(secondOperation)

        entry.removeAttachment(first.id)
        entry.removeOperation(firstOperation)

        XCTAssertEqual(entry.currentText, "draft")
        XCTAssertNil(entry.attachments[first.id])
        XCTAssertEqual(entry.attachments[second.id], second)
        XCTAssertFalse(entry.operationKeys.contains(firstOperation))
        XCTAssertTrue(entry.operationKeys.contains(secondOperation))
        XCTAssertFalse(entry.isReclaimable)
    }

    func testEntryReclamationRequiresNoContentOrDurableReferences() {
        var entry = makeEntry(text: "")
        XCTAssertTrue(entry.isReclaimable)
        let delivery = GaryxDeliveryRecordID(rawValue: "delivery")
        let feedback = GaryxFeedbackID(rawValue: "feedback")
        entry.addDeliveryReference(delivery)
        entry.addFeedbackReference(feedback)
        entry.setAliasReferenceCount(1)
        XCTAssertFalse(entry.isReclaimable)

        // Generation reset never erases historical delivery/feedback exits.
        XCTAssertTrue(entry.resetGeneration(1, barrierIdle: true, producerLive: true))
        XCTAssertEqual(entry.deliveryReferences, [delivery])
        XCTAssertEqual(entry.feedbackReferences, [feedback])
        XCTAssertFalse(entry.isReclaimable)
    }

    func testStoreIsScopePartitionedAndPromotionPreservesEntryAndToken() {
        var store = GaryxComposerPayloadStore()
        let entry = makeEntry(text: "draft")
        XCTAssertTrue(store.insert(entry))
        let originalToken = entry.lifecycle.token

        XCTAssertTrue(store.promote(entryID: entry.id, scope: scope, to: .thread("thread")))
        let promoted = store.entry(entry.id, scope: scope)
        XCTAssertEqual(promoted?.destination, .thread("thread"))
        XCTAssertEqual(promoted?.lifecycle.token, originalToken)

        let other = GaryxGatewayScope(identity: "other", epoch: 1)
        XCTAssertNil(store.entry(entry.id, scope: other))
    }

    func testPayloadLifecycleAllowsOnlyActiveToDiscardingToDiscarded() {
        var lifecycle = makeEntry().lifecycle
        XCTAssertFalse(
            lifecycle.finishDiscard(
                reservationSettled: true,
                descendantsEmpty: true,
                deliveriesSettled: true
            )
        )
        XCTAssertTrue(lifecycle.beginDiscard(discardRevision: 9))
        XCTAssertEqual(lifecycle.phase, .discarding)
        XCTAssertEqual(lifecycle.discardRevision, 9)
        XCTAssertFalse(lifecycle.beginDiscard(discardRevision: 10))
        XCTAssertFalse(
            lifecycle.finishDiscard(
                reservationSettled: false,
                descendantsEmpty: true,
                deliveriesSettled: true
            )
        )
        XCTAssertTrue(
            lifecycle.finishDiscard(
                reservationSettled: true,
                descendantsEmpty: true,
                deliveriesSettled: true
            )
        )
        XCTAssertEqual(lifecycle.phase, .discarded)
    }

    func testUnifiedMutationGateCoversEveryDurableAdmission() {
        var lifecycle = makeEntry().lifecycle
        let capture = GaryxPayloadLifecycleCapture(
            token: lifecycle.token,
            revision: lifecycle.revision
        )
        for mutation in GaryxPayloadMutationKind.allCases {
            XCTAssertEqual(
                GaryxPayloadMutationGate.admit(
                    mutation,
                    capture: capture,
                    lifecycle: lifecycle.snapshot
                ),
                .admitted,
                "mutation=\(mutation)"
            )
        }

        XCTAssertTrue(lifecycle.beginDiscard(discardRevision: 2))
        for mutation in GaryxPayloadMutationKind.allCases {
            XCTAssertEqual(
                GaryxPayloadMutationGate.admit(
                    mutation,
                    capture: capture,
                    lifecycle: lifecycle.snapshot
                ),
                .rejectedLifecycle,
                "discarding mutation=\(mutation)"
            )
        }
        XCTAssertTrue(
            lifecycle.finishDiscard(
                reservationSettled: true,
                descendantsEmpty: true,
                deliveriesSettled: true
            )
        )
        for mutation in GaryxPayloadMutationKind.allCases {
            XCTAssertEqual(
                GaryxPayloadMutationGate.admit(
                    mutation,
                    capture: capture,
                    lifecycle: lifecycle.snapshot
                ),
                .rejectedLifecycle,
                "discarded mutation=\(mutation)"
            )
        }
    }

    func testOperationCapabilityHappyPathAndIllegalTransitions() {
        var operation = makeOperation("op")
        let context = activeContext(entry: makeEntry())
        XCTAssertEqual(
            operation.transition(
                expectedKey: operation.context.key,
                to: .preparing,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .applied
        )
        XCTAssertEqual(
            operation.transition(
                expectedKey: operation.context.key,
                to: .completed,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .rejectedState
        )
        XCTAssertEqual(
            operation.transition(
                expectedKey: operation.context.key,
                to: .uploading,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .applied
        )
        XCTAssertEqual(
            operation.markUploadAttempted(
                expectedKey: operation.context.key,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .applied
        )
        XCTAssertTrue(operation.uploadAttempted)
        XCTAssertEqual(
            operation.complete(
                expectedKey: operation.context.key,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .applied
        )
        XCTAssertEqual(operation.state, .completed)
    }

    func testOperationCompletionUsesKeyScopeIdentityAndLifecycleTripleCAS() {
        let entry = makeEntry()
        let context = activeContext(entry: entry)

        var wrongKey = makeOperation("op", state: .uploading)
        XCTAssertEqual(
            wrongKey.complete(
                expectedKey: operationKey("different"),
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .rejectedKey
        )

        var wrongLifecycle = makeOperation("op", state: .uploading)
        let discarding = GaryxPayloadLifecycleSnapshot(
            token: context.lifecycle.token,
            revision: context.lifecycle.revision + 1,
            phase: .discarding
        )
        XCTAssertEqual(
            wrongLifecycle.complete(
                expectedKey: wrongLifecycle.context.key,
                lifecycle: discarding,
                scopes: context.scopes
            ),
            .rejectedLifecycle
        )
        XCTAssertEqual(
            wrongLifecycle.markUploadAttempted(
                expectedKey: wrongLifecycle.context.key,
                lifecycle: discarding,
                scopes: context.scopes
            ),
            .rejectedLifecycle
        )
        XCTAssertFalse(wrongLifecycle.uploadAttempted)

        var revokedScopes = context.scopes
        _ = revokedScopes.revoke(scope)
        var revoked = makeOperation("op", state: .uploading)
        XCTAssertEqual(
            revoked.complete(
                expectedKey: revoked.context.key,
                lifecycle: context.lifecycle,
                scopes: revokedScopes
            ),
            .rejectedScope
        )

        var invalidIdentity = makeOperation(
            "op",
            state: .uploading,
            stagedAssetID: GaryxStagedAssetID(rawValue: "asset"),
            reservedBytes: 100
        )
        invalidIdentity.invalidateIdentity()
        XCTAssertEqual(
            invalidIdentity.complete(
                expectedKey: invalidIdentity.context.key,
                lifecycle: discarding,
                scopes: context.scopes
            ),
            .archivedIdentityInvalid
        )
        XCTAssertEqual(invalidIdentity.state, .cancelled)
        XCTAssertNil(invalidIdentity.stagedAssetID)
        XCTAssertEqual(invalidIdentity.reservedBytes, 0)
    }

    func testOperationRecoveryMatrixAllStatesAttemptFlagsAndScopeLifecycles() {
        let requestedExpected: [GaryxGatewayScopeLifecycle: GaryxOperationRecoveryDecision] = [
            .active: .cancelAndCleanStaging(erasePayload: false),
            .suspended: .cancelAndCleanStaging(erasePayload: false),
            .revoked: .cancelAndCleanStaging(erasePayload: true),
        ]
        for state in [GaryxOperationCapabilityState.requested, .preparing] {
            for scopeState in [GaryxGatewayScopeLifecycle.active, .suspended, .revoked] {
                XCTAssertEqual(
                    GaryxOperationRecoveryPlanner.decide(
                        state: state,
                        uploadAttempted: false,
                        scope: scopeState
                    ),
                    requestedExpected[scopeState]
                )
            }
        }

        let uploading: [(Bool, GaryxGatewayScopeLifecycle, GaryxOperationRecoveryDecision)] = [
            (false, .active, .retryBeforeTransport),
            (false, .suspended, .suspendInOriginPartition),
            (false, .revoked, .cancelAndCleanStaging(erasePayload: true)),
            (true, .active, .failedRetryableWithFeedback),
            (true, .suspended, .failedRetryableWithFeedback),
            (true, .revoked, .scopeRevokedEvidence),
        ]
        for (attempted, scopeState, expected) in uploading {
            XCTAssertEqual(
                GaryxOperationRecoveryPlanner.decide(
                    state: .uploading,
                    uploadAttempted: attempted,
                    scope: scopeState
                ),
                expected
            )
        }

        let terminalExpected: [
            GaryxOperationCapabilityState: [GaryxGatewayScopeLifecycle: GaryxOperationRecoveryDecision]
        ] = [
            .completed: [
                .active: .placeCompletedAndCleanStaging,
                .suspended: .placeCompletedAndCleanStaging,
                .revoked: .scopeRevokedEvidence,
            ],
            .failedRetryable: [
                .active: .preserveFailedRetryable,
                .suspended: .preserveFailedRetryable,
                .revoked: .cancelAndCleanStaging(erasePayload: true),
            ],
            .failedTerminal: [
                .active: .persistFailedTerminalFeedback,
                .suspended: .persistFailedTerminalFeedback,
                .revoked: .cleanAndArchiveWithoutUI,
            ],
            .cancelled: [
                .active: .cleanOperationChild,
                .suspended: .cleanOperationChild,
                .revoked: .cleanOperationChild,
            ],
            .superseded: [
                .active: .ownershipTransferred,
                .suspended: .ownershipTransferred,
                .revoked: .settleSuccessorForRevocation,
            ],
        ]
        for (state, byScope) in terminalExpected {
            for (scopeState, expected) in byScope {
                XCTAssertEqual(
                    GaryxOperationRecoveryPlanner.decide(
                        state: state,
                        uploadAttempted: true,
                        scope: scopeState
                    ),
                    expected,
                    "state=\(state), scope=\(scopeState)"
                )
            }
        }
    }

    func testIdentityDiscardOverrideCoversEveryOperationStateWithoutResourceResidue() {
        for state in GaryxOperationCapabilityState.allCases {
            var operation = makeOperation(
                "operation-\(state.rawValue)",
                state: state,
                stagedAssetID: GaryxStagedAssetID(rawValue: "asset-\(state.rawValue)"),
                reservedBytes: 100
            )
            operation.settleIdentityDiscard()
            let expected: GaryxOperationCapabilityState = switch state {
            case .requested, .preparing, .uploading, .failedRetryable:
                .cancelled
            case .completed, .failedTerminal, .cancelled, .superseded:
                state
            }
            XCTAssertEqual(operation.state, expected, "state=\(state)")
            XCTAssertFalse(operation.identityValid, "state=\(state)")
            XCTAssertNil(operation.stagedAssetID, "state=\(state)")
            XCTAssertEqual(operation.reservedBytes, 0, "state=\(state)")
        }
    }

    func testPayloadPreparingIncludesFailedRetryableAndDoesNotAdvanceAnything() {
        let blockingStates: [GaryxOperationCapabilityState] = [
            .requested, .preparing, .uploading, .failedRetryable,
        ]
        for state in blockingStates {
            XCTAssertEqual(
                GaryxComposerSendReadinessPolicy.evaluate([makeOperation("op", state: state)]),
                .payloadPreparing
            )
        }
        for state in [
            GaryxOperationCapabilityState.completed,
            .failedTerminal,
            .cancelled,
            .superseded,
        ] {
            XCTAssertEqual(
                GaryxComposerSendReadinessPolicy.evaluate([makeOperation("op", state: state)]),
                .ready
            )
        }
    }

    func testReplacementSwapIsAtomicAndMaintainsExactlyOneFileOwner() {
        let entry = makeEntry()
        let context = activeContext(entry: entry)
        var old = makeOperation(
            "old",
            state: .failedRetryable,
            stagedAssetID: GaryxStagedAssetID(rawValue: "old-asset"),
            reservedBytes: 100
        )
        var successor = makeOperation("new")
        var record = replacement(old: old.context.key, reservation: nil)

        XCTAssertEqual(
            GaryxReplacementSwapReducer.commit(
                old: &old,
                successor: &successor,
                record: &record,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .committed
        )
        XCTAssertEqual(old.state, .superseded)
        XCTAssertEqual(old.supersededBy, successor.context.key.operationID)
        XCTAssertEqual(successor.state, .preparing)
        XCTAssertEqual(successor.stagedAssetID, record.stagedAssetID)
        XCTAssertEqual(successor.reservedBytes, record.reservedBytes)
        XCTAssertEqual(record.phase, .committed)
        XCTAssertEqual(record.newKey, successor.context.key)

        var invalidOld = makeOperation("invalid", state: .completed)
        var untouchedSuccessor = makeOperation("untouched")
        var untouchedRecord = replacement(old: invalidOld.context.key, reservation: nil)
        let beforeOld = invalidOld
        let beforeSuccessor = untouchedSuccessor
        let beforeRecord = untouchedRecord
        XCTAssertEqual(
            GaryxReplacementSwapReducer.commit(
                old: &invalidOld,
                successor: &untouchedSuccessor,
                record: &untouchedRecord,
                lifecycle: context.lifecycle,
                scopes: context.scopes
            ),
            .rejectedOldOperation
        )
        XCTAssertEqual(invalidOld, beforeOld)
        XCTAssertEqual(untouchedSuccessor, beforeSuccessor)
        XCTAssertEqual(untouchedRecord, beforeRecord)
    }

    func testReplacementRecoveryAndSixReclamationRows() {
        let old = operationKey("old")
        var pending = replacement(old: old, reservation: nil)
        XCTAssertEqual(
            GaryxReplacementPlanner.recover(pending),
            .abortReleaseQuotaAndDeleteProvisional
        )
        pending.abort()
        XCTAssertEqual(GaryxReplacementPlanner.recover(pending), .garbageCollect)

        let reserved = GaryxSendReservationID(rawValue: 9)
        let reservedOld = operationKey("old-reserved", reservation: reserved)
        var committed = replacement(old: reservedOld, reservation: reserved)
        let successor = operationKey("new", reservation: reserved)
        committed.commit(newKey: successor)
        XCTAssertEqual(
            GaryxReplacementPlanner.recover(committed),
            .restoreSuccessor(successor)
        )

        var malformed = replacement(old: old, reservation: reserved)
        malformed.commit(newKey: operationKey("wrong-scope"))
        XCTAssertEqual(malformed.phase, .pendingReplacement)
        XCTAssertEqual(
            GaryxReplacementPlanner.recover(malformed),
            .abortReleaseQuotaAndDeleteProvisional
        )

        XCTAssertEqual(GaryxReplacementPlanner.reclaim(successorState: .completed, scope: .active), .reclaim)
        XCTAssertEqual(GaryxReplacementPlanner.reclaim(successorState: .failedTerminal, scope: .active), .reclaim)
        XCTAssertEqual(GaryxReplacementPlanner.reclaim(successorState: .cancelled, scope: .active), .reclaim)
        XCTAssertEqual(
            GaryxReplacementPlanner.reclaim(successorState: .superseded, scope: .active),
            .awaitSuccessorOwnerTransaction
        )
        XCTAssertEqual(
            GaryxReplacementPlanner.reclaim(successorState: .failedRetryable, scope: .active),
            .retainActiveManifest
        )
        XCTAssertEqual(
            GaryxReplacementPlanner.reclaim(successorState: .failedRetryable, scope: .revoked),
            .reclaim
        )
    }

    func testFeedbackOnlyPresentsForMatchingEntryWithInteractionOwner() {
        var feedback = GaryxOperationFeedback(
            id: GaryxFeedbackID(rawValue: "feedback"),
            scope: scope,
            entryID: entryID,
            operationID: GaryxOperationID(rawValue: "op"),
            kind: .uploadRetryable
        )
        XCTAssertFalse(feedback.present(hostEntryID: GaryxComposerPayloadEntryID(rawValue: "B"), hasInteractionOwner: true))
        XCTAssertFalse(feedback.present(hostEntryID: entryID, hasInteractionOwner: false))
        XCTAssertTrue(feedback.present(hostEntryID: entryID, hasInteractionOwner: true))
        XCTAssertEqual(feedback.phase, .presented)
        feedback.acknowledge()
        XCTAssertEqual(feedback.phase, .acknowledged)
        feedback.archive()
        XCTAssertEqual(feedback.phase, .acknowledged, "terminal feedback cannot change twice")

        var revoked = GaryxOperationFeedback(
            id: GaryxFeedbackID(rawValue: "revoked"),
            scope: scope,
            entryID: entryID,
            operationID: nil,
            kind: .uploadTerminal
        )
        revoked.archive()
        XCTAssertTrue(revoked.isTerminal)
    }

    func testFailedTerminalLineageAdmitsFreshOperationUntilFeedbackIsTerminal() {
        let entry = makeEntry()
        let lineageID = GaryxAttachmentLineageID(rawValue: "lineage")
        let feedbackID = GaryxFeedbackID(rawValue: "feedback-lineage")
        var feedback = GaryxOperationFeedback(
            id: feedbackID,
            scope: scope,
            entryID: entry.id,
            operationID: GaryxOperationID(rawValue: "failed"),
            lineageID: lineageID,
            kind: .uploadTerminal
        )
        var lineage = GaryxAttachmentLineageTombstone(
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
        let fresh = makeOperation("fresh")

        XCTAssertTrue(
            lineage.admitsFreshOperation(
                fresh,
                feedback: feedback,
                lifecycle: entry.lifecycle.snapshot
            )
        )
        XCTAssertFalse(
            lineage.admitsFreshOperation(
                fresh,
                feedback: feedback,
                lifecycle: .init(
                    token: entry.lifecycle.token,
                    revision: entry.lifecycle.revision + 1,
                    phase: .discarding
                )
            )
        )
        XCTAssertFalse(lineage.release(after: feedback))
        feedback.acknowledge()
        XCTAssertTrue(lineage.release(after: feedback))
        XCTAssertEqual(lineage.phase, .released)
        XCTAssertFalse(
            lineage.admitsFreshOperation(
                fresh,
                feedback: feedback,
                lifecycle: entry.lifecycle.snapshot
            )
        )
    }

    func testConflictSetAdmissionIsFailClosedAndPreservesThreeCandidates() {
        var set = GaryxPayloadConflictSet(
            id: GaryxPayloadConflictSetID(rawValue: "conflict"),
            scope: scope
        )
        let rejected = GaryxPayloadConflictCandidate(
            entryID: GaryxComposerPayloadEntryID(rawValue: "rejected"),
            label: "Rejected"
        )
        XCTAssertFalse(set.admitCandidate(rejected, membershipDurabilityAvailable: false))
        XCTAssertTrue(set.candidates.isEmpty, "domain promotion must remain uncommitted")

        for index in 1...3 {
            XCTAssertTrue(
                set.admitCandidate(
                    GaryxPayloadConflictCandidate(
                        entryID: GaryxComposerPayloadEntryID(rawValue: "entry-\(index)"),
                        label: "Candidate \(index)"
                    ),
                    membershipDurabilityAvailable: true
                )
            )
        }
        XCTAssertEqual(set.candidates.count, 3)
        XCTAssertTrue(set.pendingDecision)
        for candidate in set.candidates.map(\.entryID) {
            set.resolve(entryID: candidate)
        }
        XCTAssertFalse(set.pendingDecision)
    }

    func testIdentityFiveEventsHaveDistinctDestructiveAuthority() throws {
        var store = GaryxComposerPayloadStore()
        let entry = makeEntry(text: "draft")
        XCTAssertTrue(store.insert(entry))

        XCTAssertEqual(
            GaryxPayloadIdentityReducer.apply(
                .aliasSourceRetired(draftID: "draft"),
                scope: scope,
                store: &store
            ),
            .aliasOnly
        )
        XCTAssertEqual(store.entry(entry.id, scope: scope)?.lifecycle.phase, .active)
        XCTAssertEqual(
            GaryxPayloadIdentityReducer.apply(
                .routeOccurrenceSuperseded(GaryxRouteInstanceID(rawValue: "occurrence")),
                scope: scope,
                store: &store
            ),
            .occurrenceOnly
        )
        XCTAssertEqual(store.entry(entry.id, scope: scope)?.lifecycle.phase, .active)

        XCTAssertEqual(
            GaryxPayloadIdentityReducer.apply(
                .payloadGenerationReset(
                    entryID: entry.id,
                    generation: 1,
                    barrierIdle: false,
                    producerLive: true
                ),
                scope: scope,
                store: &store
            ),
            .rejected
        )
        XCTAssertEqual(store.entry(entry.id, scope: scope)?.currentText, "draft")
        XCTAssertEqual(
            GaryxPayloadIdentityReducer.apply(
                .payloadGenerationReset(
                    entryID: entry.id,
                    generation: 1,
                    barrierIdle: true,
                    producerLive: true
                ),
                scope: scope,
                store: &store
            ),
            .generationReset
        )
        XCTAssertEqual(store.entry(entry.id, scope: scope)?.lifecycle.phase, .active)

        XCTAssertEqual(
            GaryxPayloadIdentityReducer.apply(
                .destinationDiscarded(.draft("draft"), revision: 10),
                scope: scope,
                store: &store
            ),
            .beganDiscard([entry.id])
        )
        XCTAssertEqual(store.entry(entry.id, scope: scope)?.lifecycle.phase, .discarding)

        var secondStore = GaryxComposerPayloadStore()
        let second = makeEntry(id: "second", text: "other")
        XCTAssertTrue(secondStore.insert(second))
        XCTAssertEqual(
            GaryxPayloadIdentityReducer.apply(
                .payloadEntryDiscarded(second.id, revision: 11),
                scope: scope,
                store: &secondStore
            ),
            .beganDiscard([second.id])
        )
        XCTAssertEqual(secondStore.entry(second.id, scope: scope)?.lifecycle.phase, .discarding)
    }

    func testDestinationAndEntryDiscardRejectAllAdmissionKindsWhileAliasAndOccurrenceDoNot() throws {
        for destructive in [false, true] {
            var store = GaryxComposerPayloadStore()
            let entry = makeEntry()
            XCTAssertTrue(store.insert(entry))
            let capture = GaryxPayloadLifecycleCapture(
                token: entry.lifecycle.token,
                revision: entry.lifecycle.revision
            )
            if destructive {
                _ = GaryxPayloadIdentityReducer.apply(
                    .payloadEntryDiscarded(entry.id, revision: 2),
                    scope: scope,
                    store: &store
                )
            } else {
                _ = GaryxPayloadIdentityReducer.apply(
                    .routeOccurrenceSuperseded(GaryxRouteInstanceID(rawValue: "occ")),
                    scope: scope,
                    store: &store
                )
            }
            let lifecycle = try XCTUnwrap(store.entry(entry.id, scope: scope)?.lifecycle.snapshot)
            for kind in GaryxPayloadMutationKind.allCases {
                XCTAssertEqual(
                    GaryxPayloadMutationGate.admit(kind, capture: capture, lifecycle: lifecycle),
                    destructive ? .rejectedLifecycle : .admitted,
                    "destructive=\(destructive), kind=\(kind)"
                )
            }
        }
    }

    private var entryID: GaryxComposerPayloadEntryID {
        GaryxComposerPayloadEntryID(rawValue: "entry")
    }

    private func makeEntry(id: String = "entry", text: String = "") -> GaryxComposerPayloadEntry {
        let entryID = GaryxComposerPayloadEntryID(rawValue: id)
        return GaryxComposerPayloadEntry(
            id: entryID,
            scope: scope,
            destination: .draft("draft"),
            lifecycleToken: GaryxPayloadLifecycleToken(entryID: entryID, nonce: "token-\(id)"),
            currentGeneration: 1,
            text: text
        )
    }

    private func attachment(_ id: String, generation: UInt64) -> GaryxComposerAttachment {
        GaryxComposerAttachment(
            id: GaryxAttachmentID(rawValue: id),
            stagedAssetID: GaryxStagedAssetID(rawValue: "asset-\(id)"),
            generation: generation,
            byteCount: 10
        )
    }

    private func operationKey(
        _ id: String,
        reservation: GaryxSendReservationID? = nil
    ) -> GaryxOperationCapabilityKey {
        GaryxOperationCapabilityKey(
            scope: scope,
            entryID: entryID,
            generation: 1,
            reservationID: reservation,
            branch: .followup,
            operationID: GaryxOperationID(rawValue: id)
        )
    }

    private func makeOperation(
        _ id: String,
        state: GaryxOperationCapabilityState = .requested,
        stagedAssetID: GaryxStagedAssetID? = nil,
        reservedBytes: Int = 0
    ) -> GaryxOperationCapability {
        let entry = makeEntry()
        let key = operationKey(id)
        return GaryxOperationCapability(
            context: GaryxScopeBoundOperationContext(
                key: key,
                clientIdentity: "client-gateway",
                configurationFingerprint: "configuration-1",
                payloadLifecycle: GaryxPayloadLifecycleCapture(
                    token: entry.lifecycle.token,
                    revision: entry.lifecycle.revision
                )
            ),
            state: state,
            stagedAssetID: stagedAssetID,
            reservedBytes: reservedBytes
        )
    }

    private func activeContext(
        entry: GaryxComposerPayloadEntry
    ) -> (lifecycle: GaryxPayloadLifecycleSnapshot, scopes: GaryxGatewayScopeRegistry) {
        (
            entry.lifecycle.snapshot,
            GaryxGatewayScopeRegistry(initialActiveScope: scope)
        )
    }

    private func replacement(
        old: GaryxOperationCapabilityKey,
        reservation: GaryxSendReservationID?
    ) -> GaryxReplacementRecord {
        GaryxReplacementRecord(
            id: GaryxReplacementID(rawValue: "replacement-\(old.operationID.rawValue)"),
            scope: scope,
            entryID: entryID,
            oldKey: old,
            reservationID: reservation,
            branch: .followup,
            stagedAssetID: GaryxStagedAssetID(rawValue: "replacement-asset"),
            reservedBytes: 200
        )
    }
}
