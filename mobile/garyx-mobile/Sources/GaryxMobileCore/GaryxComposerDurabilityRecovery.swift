import Foundation

public enum GaryxDurableDeliveryRecoveryDisposition: String, Codable, Sendable {
    case acknowledged
    case safeToRetry
    case userTerminable
    case terminal
}

public struct GaryxComposerDurabilityRecoveryReport: Equatable, Sendable {
    public var syntheticReservationRecoveries: Int
    public var operationSettlements: Int
    public var replacementSettlements: Int
    public var discardSettlements: Int
    public var createSettlements: Int
    public var undispatchedDeliverySettlements: Int
    public var deliveryDispositions: [
        GaryxDeliveryRecordID: GaryxDurableDeliveryRecoveryDisposition
    ]
    public var retryableOperationKeys: Set<GaryxOperationCapabilityKey>

    public init(
        syntheticReservationRecoveries: Int = 0,
        operationSettlements: Int = 0,
        replacementSettlements: Int = 0,
        discardSettlements: Int = 0,
        createSettlements: Int = 0,
        undispatchedDeliverySettlements: Int = 0,
        deliveryDispositions: [
            GaryxDeliveryRecordID: GaryxDurableDeliveryRecoveryDisposition
        ] = [:],
        retryableOperationKeys: Set<GaryxOperationCapabilityKey> = []
    ) {
        self.syntheticReservationRecoveries = syntheticReservationRecoveries
        self.operationSettlements = operationSettlements
        self.replacementSettlements = replacementSettlements
        self.discardSettlements = discardSettlements
        self.createSettlements = createSettlements
        self.undispatchedDeliverySettlements = undispatchedDeliverySettlements
        self.deliveryDispositions = deliveryDispositions
        self.retryableOperationKeys = retryableOperationKeys
    }
}

public enum GaryxComposerDurabilityRecoveryError: Error, Equatable, Sendable {
    case syntheticReservationCannotConverge(GaryxReservationLedgerKey)
    case operationEntryMissing(GaryxOperationCapabilityKey)
    case undispatchedDeliveryCannotConverge(GaryxDeliveryRecordID)
    case recoveryDidNotConverge
}

/// Launch-time convergence for A4d-1. Each orthogonal settlement is its own
/// durability transaction, so death after any commit simply resumes at the
/// next persisted state. No network action is performed here.
public actor GaryxComposerDurabilityLaunchRecovery {
    private let durability: any GaryxComposerDurabilityStore
    private let staging: GaryxComposerStagedAssetStore?
    private let scopes: GaryxGatewayScopeRegistry

    public init(
        durability: any GaryxComposerDurabilityStore,
        staging: GaryxComposerStagedAssetStore? = nil,
        scopes: GaryxGatewayScopeRegistry
    ) {
        self.durability = durability
        self.staging = staging
        self.scopes = scopes
    }

    public func recover() async throws -> GaryxComposerDurabilityRecoveryReport {
        var report = GaryxComposerDurabilityRecoveryReport()
        for _ in 0..<10_000 {
            if try await recoverOneDiscardStep() {
                report.discardSettlements += 1
                if let staging { _ = try await staging.settleCondemnedFiles() }
                continue
            }
            if try await recoverOneSyntheticReservation() {
                report.syntheticReservationRecoveries += 1
                continue
            }
            if try await recoverOneReplacement() {
                report.replacementSettlements += 1
                if let staging { _ = try await staging.settleCondemnedFiles() }
                continue
            }
            if try await recoverOneOperation() {
                report.operationSettlements += 1
                if let staging { _ = try await staging.settleCondemnedFiles() }
                continue
            }
            if try await recoverOneTransportAttemptedDelivery() {
                continue
            }
            if try await recoverOneCreateDelivery() {
                report.createSettlements += 1
                continue
            }
            if try await recoverOneDeferredDraft() {
                continue
            }
            if try await recoverOneUndispatchedDelivery() {
                report.undispatchedDeliverySettlements += 1
                continue
            }
            if let staging {
                let before = try await durability.load()
                if !before.pendingFileCleanup.isEmpty {
                    _ = try await staging.settleCondemnedFiles()
                    continue
                }
            }
            let snapshot = try await durability.load()
            report.deliveryDispositions = Dictionary(
                uniqueKeysWithValues: snapshot.deliveries.map { id, delivery in
                    (id, Self.deliveryDisposition(delivery, snapshot: snapshot))
                }
            )
            report.retryableOperationKeys = Set(snapshot.operations.compactMap { key, operation in
                operation.state == .failedRetryable ? key : nil
            })
            return report
        }
        throw GaryxComposerDurabilityRecoveryError.recoveryDidNotConverge
    }

    /// Crossing the durable attempt gate proves only that transport may have
    /// started. A process boundary destroys the in-memory response outcome, so
    /// the persisted record must become an actionable ambiguous delivery.
    private func recoverOneTransportAttemptedDelivery() async throws -> Bool {
        let snapshot = try await durability.load()
        guard var delivery = snapshot.deliveries.values
            .filter({ $0.phase == .transportAttempted })
            .sorted(by: { $0.id.rawValue < $1.id.rawValue })
            .first,
              delivery.markAmbiguous() else {
            return false
        }
        _ = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "recover attempted delivery as user-terminable ambiguous",
                mutations: [.upsertDelivery(delivery)]
            )
        )
        return true
    }

    /// A process boundary destroys the only in-memory proof that a multi-stage
    /// create request did not cross its current transport edge. Every
    /// unacknowledged stage therefore becomes honestly ambiguous on relaunch;
    /// no gateway uniqueness/query capability is assumed.
    private func recoverOneCreateDelivery() async throws -> Bool {
        let snapshot = try await durability.load()
        guard var state = snapshot.createDeliveries.values
            .filter({ $0.phase != .ambiguous && $0.phase != .acknowledged })
            .sorted(by: {
                if $0.scope.identity != $1.scope.identity {
                    return $0.scope.identity < $1.scope.identity
                }
                if $0.scope.epoch != $1.scope.epoch {
                    return $0.scope.epoch < $1.scope.epoch
                }
                return $0.createIntentID < $1.createIntentID
            })
            .first else {
            return false
        }
        state.responseLost()
        _ = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "recover multi-stage create as ambiguous",
                mutations: [.upsertCreateDelivery(state)]
            )
        )
        return true
    }

    /// A bare message that never crossed the durable attempt gate is known not
    /// to have reached transport. Restore it through automatic composer
    /// placement and terminalize its outbox record in one transaction,
    /// reclaiming quota without risking a duplicate send. Multi-stage creates
    /// keep their own honest create-ambiguity exit instead.
    private func recoverOneUndispatchedDelivery() async throws -> Bool {
        let beforeAllocation = try await durability.load()
        guard let candidate = beforeAllocation.deliveries.values
            .filter({
                $0.phase == .notDispatched
                    && $0.userDisposition == .none
                    && !GaryxUndispatchedDeliveryRecoveryPlanner.isOwnedByCreate(
                        $0,
                        snapshot: beforeAllocation
                    )
            })
            .sorted(by: { $0.id.rawValue < $1.id.rawValue })
            .first else {
            return false
        }
        let recoveredGeneration = try await durability.allocatePayloadGeneration()
        let snapshot = try await durability.load()
        guard let current = snapshot.deliveries[candidate.id] else {
            return true
        }
        guard current.phase == .notDispatched,
              current.userDisposition == .none else {
            return true
        }
        guard let plan = GaryxUndispatchedDeliveryRecoveryPlanner.automaticPlan(
            snapshot: snapshot,
            deliveryID: candidate.id,
            recoveredGeneration: recoveredGeneration
        ) else {
            throw GaryxComposerDurabilityRecoveryError
                .undispatchedDeliveryCannotConverge(candidate.id)
        }
        _ = try await durability.commit(plan.transaction)
        return true
    }

    /// A recovered payload deferred behind newer composer input remains
    /// durable and invisible until its host is empty. Relaunch then adopts the
    /// earliest eligible payload without presenting a choice or overwriting the
    /// user's current intent.
    private func recoverOneDeferredDraft() async throws -> Bool {
        let beforeAllocation = try await durability.load()
        guard let candidate = GaryxDeferredDraftAdoptionPlanner.candidate(
            snapshot: beforeAllocation
        ) else {
            return false
        }
        let replacementGeneration = try await durability.allocatePayloadGeneration()
        let snapshot = try await durability.load()
        guard let plan = GaryxDeferredDraftAdoptionPlanner.plan(
            snapshot: snapshot,
            candidate: candidate,
            replacementGeneration: replacementGeneration
        ) else {
            return true
        }
        _ = try await durability.commit(plan.transaction)
        return true
    }

    private func recoverOneSyntheticReservation() async throws -> Bool {
        var snapshot = try await durability.load()
        guard let ledger = snapshot.ledgers.values
            .filter({ $0.terminalOutcome == nil })
            .sorted(by: Self.ledgerSort)
            .first else {
            return false
        }
        let mergeGeneration = try await durability.allocatePayloadGeneration()
        snapshot = try await durability.load()
        guard let plan = GaryxSyntheticReservationRecoveryPlanner.plan(
            snapshot: snapshot,
            ledgerKey: ledger.key,
            mergeGeneration: mergeGeneration,
            conflictSetID: GaryxPayloadConflictSetID(
                rawValue: "launch-recovery-\(ledger.key.entryID.rawValue)-\(ledger.key.reservationID.rawValue)"
            )
        ) else {
            throw GaryxComposerDurabilityRecoveryError.syntheticReservationCannotConverge(
                ledger.key
            )
        }
        _ = try await durability.commit(plan.transaction)
        return true
    }

    private func recoverOneOperation() async throws -> Bool {
        let snapshot = try await durability.load()
        let keys = Set(snapshot.operations.keys).union(snapshot.manifests.keys).sorted {
            Self.operationKeySort($0, $1)
        }
        for key in keys {
            let lifecycle = scopes.lifecycle(of: key.scope)
            if lifecycle == .revoked {
                if Self.revokedEntryRequiresPayloadErase(
                    scope: key.scope,
                    entryID: key.entryID,
                    snapshot: snapshot
                ) {
                    try await cleanRevokedEntry(
                        scope: key.scope,
                        entryID: key.entryID,
                        snapshot: snapshot
                    )
                } else {
                    try await cleanRevokedOperationChildren(
                        scope: key.scope,
                        entryID: key.entryID,
                        triggerKey: key,
                        snapshot: snapshot
                    )
                }
                return true
            }
            guard let operation = snapshot.operations[key] else {
                // A manifest without its capability cannot own a network
                // action; archive it, its Entry membership, and its file in
                // one transaction before continuing.
                guard var entry = snapshot.payloadStore.entry(key.entryID, scope: key.scope) else {
                    throw GaryxComposerDurabilityRecoveryError.operationEntryMissing(key)
                }
                entry.removeOperation(key)
                var mutations: [GaryxComposerDurabilityMutation] = [
                    .removeManifest(key),
                    .upsertEntry(entry),
                ]
                if let assetID = snapshot.stagedAssetOwners.first(where: { $0.value == key })?.key {
                    mutations.insert(.registerFileCleanup(assetID: assetID, owner: key), at: 0)
                    mutations.insert(.releaseStagedAsset(assetID), at: 1)
                }
                _ = try await durability.commit(
                    .init(
                        expectedRevision: snapshot.revision,
                        label: "archive ownerless operation manifest",
                        mutations: mutations
                    )
                )
                return true
            }
            if let reservationID = key.reservationID {
                let ledgerKey = GaryxReservationLedgerKey(
                    scope: key.scope,
                    entryID: key.entryID,
                    reservationID: reservationID
                )
                if let targetGeneration = snapshot.ledgers[ledgerKey]?.targetMapping?.generation,
                   targetGeneration != key.generation {
                    try await remapOperation(
                        operation,
                        manifest: snapshot.manifests[key],
                        targetGeneration: targetGeneration,
                        snapshot: snapshot
                    )
                    return true
                }
            }
            guard var entry = snapshot.payloadStore.entry(key.entryID, scope: key.scope) else {
                throw GaryxComposerDurabilityRecoveryError.operationEntryMissing(key)
            }
            let manifest = snapshot.manifests[key]
            let state = manifest?.state ?? operation.state
            let attempted = manifest?.uploadAttempted ?? operation.uploadAttempted
            let decision = GaryxOperationRecoveryPlanner.decide(
                state: state,
                uploadAttempted: attempted,
                scope: lifecycle
            )

            switch decision {
            case .retryBeforeTransport, .suspendInOriginPartition,
                 .preserveFailedRetryable:
                continue
            case .failedRetryableWithFeedback:
                var next = operation
                let registry = Self.registry(for: key.scope, lifecycle: lifecycle)
                guard next.transition(
                    expectedKey: key,
                    to: .failedRetryable,
                    lifecycle: entry.lifecycle.snapshot,
                    scopes: registry
                ) == .applied else {
                    continue
                }
                let feedbackID = Self.recoveryFeedbackID(key: key, kind: "retryable")
                let feedback = GaryxOperationFeedback(
                    id: feedbackID,
                    scope: key.scope,
                    entryID: key.entryID,
                    operationID: key.operationID,
                    kind: .uploadRetryable
                )
                entry.addFeedbackReference(feedbackID)
                _ = try await durability.commit(
                    .init(
                        expectedRevision: snapshot.revision,
                        label: "recover attempted upload as failed-retryable",
                        mutations: [
                            .upsertOperation(next),
                            .upsertManifest(
                                GaryxOperationManifest(
                                    key: key,
                                    stagedPath: manifest?.stagedPath ?? "",
                                    state: .failedRetryable,
                                    uploadAttempted: true
                                )
                            ),
                            .upsertFeedback(feedback),
                            .upsertEntry(entry),
                        ]
                    )
                )
                return true
            case .persistFailedTerminalFeedback:
                let feedbackID = Self.recoveryFeedbackID(key: key, kind: "terminal")
                if snapshot.feedback[feedbackID] != nil,
                   manifest == nil,
                   operation.stagedAssetID.map({ snapshot.stagedAssetOwners[$0] == nil }) ?? true {
                    continue
                }
                let feedback = snapshot.feedback[feedbackID] ?? GaryxOperationFeedback(
                    id: feedbackID,
                    scope: key.scope,
                    entryID: key.entryID,
                    operationID: key.operationID,
                    kind: .uploadTerminal
                )
                entry.addFeedbackReference(feedbackID)
                var mutations: [GaryxComposerDurabilityMutation] = [
                    .upsertFeedback(feedback),
                    .upsertEntry(entry),
                    .removeManifest(key),
                ]
                if let assetID = operation.stagedAssetID {
                    if snapshot.pendingFileCleanup[assetID] == nil {
                        mutations.append(.registerFileCleanup(assetID: assetID, owner: key))
                    }
                    if snapshot.stagedAssetOwners[assetID] == key {
                        mutations.append(.releaseStagedAsset(assetID))
                    }
                }
                _ = try await durability.commit(
                    .init(
                        expectedRevision: snapshot.revision,
                        label: "recover failed-terminal feedback and cleanup",
                        mutations: mutations
                    )
                )
                return true
            case .cancelAndCleanStaging, .archiveAttemptedUploadEvidence,
                 .archiveCompletedPayloadEvidence, .placeCompletedAndCleanStaging,
                 .cleanAndArchiveWithoutUI, .cleanOperationChild,
                 .ownershipTransferred, .settleSuccessorForRevocation:
                try await cleanOperation(
                    operation,
                    entry: &entry,
                    snapshot: snapshot
                )
                return true
            }
        }
        return false
    }

    private func remapOperation(
        _ operation: GaryxOperationCapability,
        manifest: GaryxOperationManifest?,
        targetGeneration: UInt64,
        snapshot: GaryxComposerDurabilitySnapshot
    ) async throws {
        let oldKey = operation.context.key
        let newKey = oldKey.remapped(toGeneration: targetGeneration)
        guard var entry = snapshot.payloadStore.entry(oldKey.entryID, scope: oldKey.scope) else {
            throw GaryxComposerDurabilityRecoveryError.operationEntryMissing(oldKey)
        }
        entry.remapOperationKey(from: oldKey, to: newKey)
        var mutations: [GaryxComposerDurabilityMutation] = [
            .removeManifest(oldKey),
            .removeOperation(oldKey),
            .upsertEntry(entry),
            .upsertOperation(operation.remapped(toGeneration: targetGeneration)),
        ]
        if let manifest {
            mutations.append(.upsertManifest(manifest.remapped(toGeneration: targetGeneration)))
        }
        if let assetID = operation.stagedAssetID,
           snapshot.stagedAssetOwners[assetID] == oldKey {
            mutations.append(.releaseStagedAsset(assetID))
            mutations.append(
                .reserveStagedAsset(
                    assetID: assetID,
                    owner: newKey,
                    bytes: snapshot.stagedAssetReservedBytes[assetID] ?? operation.reservedBytes
                )
            )
        }
        for replacement in snapshot.replacements.values where
            replacement.oldKey == oldKey || replacement.newKey == oldKey {
            mutations.append(
                .upsertReplacement(replacement.remapped(from: oldKey, to: newKey))
            )
        }
        _ = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "recover operation reservation target mapping",
                mutations: mutations
            )
        )
    }

    private func cleanOperation(
        _ original: GaryxOperationCapability,
        entry: inout GaryxComposerPayloadEntry,
        snapshot: GaryxComposerDurabilitySnapshot
    ) async throws {
        let key = original.context.key
        var operation = original
        operation.settleIdentityDiscard()
        entry.removeOperation(key)
        var mutations: [GaryxComposerDurabilityMutation] = [.upsertOperation(operation)]
        if let assetID = original.stagedAssetID {
            if snapshot.pendingFileCleanup[assetID] == nil {
                mutations.append(.registerFileCleanup(assetID: assetID, owner: key))
            }
            if snapshot.stagedAssetOwners[assetID] == key {
                mutations.append(.releaseStagedAsset(assetID))
            }
        }
        mutations.append(.removeManifest(key))
        mutations.append(.removeOperation(key))
        mutations.append(.upsertEntry(entry))
        _ = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "settle operation recovery",
                mutations: mutations
            )
        )
    }

    /// Revocation removes a shared Entry only after every operation descendant
    /// and staged owner in that Entry has been settled in the same transaction.
    /// Removing one child and the Entry separately would invalidate siblings
    /// and brick every subsequent launch on the same durable snapshot.
    private func cleanRevokedEntry(
        scope: GaryxGatewayScope,
        entryID: GaryxComposerPayloadEntryID,
        snapshot: GaryxComposerDurabilitySnapshot
    ) async throws {
        let keys = Set(snapshot.operations.keys)
            .union(snapshot.manifests.keys)
            .filter { $0.scope == scope && $0.entryID == entryID }
        var mutations: [GaryxComposerDurabilityMutation] = []
        var newlyCondemned: Set<GaryxStagedAssetID> = []

        func appendAssetCleanup(
            assetID: GaryxStagedAssetID,
            owner: GaryxOperationCapabilityKey
        ) {
            if snapshot.pendingFileCleanup[assetID] == nil,
               newlyCondemned.insert(assetID).inserted {
                mutations.append(.registerFileCleanup(assetID: assetID, owner: owner))
            }
            if snapshot.stagedAssetOwners[assetID] != nil {
                mutations.append(.releaseStagedAsset(assetID))
            }
        }

        for key in keys {
            if let assetID = snapshot.operations[key]?.stagedAssetID {
                appendAssetCleanup(assetID: assetID, owner: key)
            }
            mutations.append(.removeManifest(key))
            mutations.append(.removeOperation(key))
        }
        for replacement in snapshot.replacements.values where
            replacement.scope == scope && replacement.entryID == entryID {
            let owner = snapshot.stagedAssetOwners[replacement.stagedAssetID]
                ?? replacement.newKey
                ?? replacement.oldKey
            appendAssetCleanup(assetID: replacement.stagedAssetID, owner: owner)
            mutations.append(.removeReplacement(replacement.id))
        }
        for feedback in snapshot.feedback.values where
            feedback.scope == scope && feedback.entryID == entryID {
            mutations.append(.removeFeedback(feedback.id))
        }
        for lineage in snapshot.attachmentLineages.values where
            lineage.scope == scope && lineage.entryID == entryID {
            mutations.append(.removeAttachmentLineage(lineage.id))
        }
        mutations.append(.removeEntry(scope: scope, entryID: entryID))

        _ = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "settle revoked Entry descendants atomically",
                mutations: mutations
            )
        )
    }

    /// Cancelled and failed-retryable rows use the matrix's child-only revoke
    /// rule: their operation resources are removed without deleting sibling
    /// text or attachments from the shared Entry.
    private func cleanRevokedOperationChildren(
        scope: GaryxGatewayScope,
        entryID: GaryxComposerPayloadEntryID,
        triggerKey: GaryxOperationCapabilityKey,
        snapshot: GaryxComposerDurabilitySnapshot
    ) async throws {
        guard var entry = snapshot.payloadStore.entry(entryID, scope: scope) else {
            throw GaryxComposerDurabilityRecoveryError.operationEntryMissing(triggerKey)
        }
        let keys = Set(snapshot.operations.keys)
            .union(snapshot.manifests.keys)
            .filter { $0.scope == scope && $0.entryID == entryID }
        let operationIDs = Set(keys.map(\.operationID))
        var mutations: [GaryxComposerDurabilityMutation] = []
        var newlyCondemned: Set<GaryxStagedAssetID> = []

        func appendAssetCleanup(
            assetID: GaryxStagedAssetID,
            owner: GaryxOperationCapabilityKey
        ) {
            if snapshot.pendingFileCleanup[assetID] == nil,
               newlyCondemned.insert(assetID).inserted {
                mutations.append(.registerFileCleanup(assetID: assetID, owner: owner))
            }
            if snapshot.stagedAssetOwners[assetID] != nil {
                mutations.append(.releaseStagedAsset(assetID))
            }
        }

        for key in keys {
            if let assetID = snapshot.operations[key]?.stagedAssetID {
                appendAssetCleanup(assetID: assetID, owner: key)
            }
            entry.removeOperation(key)
            mutations.append(.removeManifest(key))
            mutations.append(.removeOperation(key))
        }

        for replacement in snapshot.replacements.values where
            replacement.scope == scope && replacement.entryID == entryID {
            let owner = snapshot.stagedAssetOwners[replacement.stagedAssetID]
                ?? replacement.newKey
                ?? replacement.oldKey
            appendAssetCleanup(assetID: replacement.stagedAssetID, owner: owner)
            mutations.append(.removeReplacement(replacement.id))
        }

        let removedFeedback = snapshot.feedback.values.filter {
            $0.scope == scope
                && $0.entryID == entryID
                && $0.operationID.map(operationIDs.contains) == true
        }
        let removedFeedbackIDs = Set(removedFeedback.map { $0.id })
        for lineage in snapshot.attachmentLineages.values where
            removedFeedbackIDs.contains(lineage.feedbackID) {
            mutations.append(.removeAttachmentLineage(lineage.id))
        }
        for feedback in removedFeedback {
            entry.removeFeedbackReference(feedback.id)
            mutations.append(.removeFeedback(feedback.id))
        }
        mutations.append(.upsertEntry(entry))

        _ = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "settle revoked operation children without erasing Entry",
                mutations: mutations
            )
        )
    }

    private static func revokedEntryRequiresPayloadErase(
        scope: GaryxGatewayScope,
        entryID: GaryxComposerPayloadEntryID,
        snapshot: GaryxComposerDurabilitySnapshot
    ) -> Bool {
        Set(snapshot.operations.keys).union(snapshot.manifests.keys).contains { key in
            guard key.scope == scope, key.entryID == entryID else { return false }
            switch snapshot.manifests[key]?.state ?? snapshot.operations[key]?.state {
            case .cancelled, .failedRetryable, .superseded:
                return false
            case .requested, .preparing, .uploading, .completed,
                 .failedTerminal, .none:
                return true
            }
        }
    }

    private func recoverOneReplacement() async throws -> Bool {
        let snapshot = try await durability.load()
        for record in snapshot.replacements.values.sorted(by: { $0.id.rawValue < $1.id.rawValue }) {
            let lifecycle = scopes.lifecycle(of: record.scope)
            if lifecycle == .revoked,
               snapshot.payloadStore.entry(record.entryID, scope: record.scope) != nil {
                if Self.revokedEntryRequiresPayloadErase(
                    scope: record.scope,
                    entryID: record.entryID,
                    snapshot: snapshot
                ) {
                    try await cleanRevokedEntry(
                        scope: record.scope,
                        entryID: record.entryID,
                        snapshot: snapshot
                    )
                } else {
                    try await cleanRevokedOperationChildren(
                        scope: record.scope,
                        entryID: record.entryID,
                        triggerKey: record.newKey ?? record.oldKey,
                        snapshot: snapshot
                    )
                }
                return true
            }
            switch GaryxReplacementPlanner.recover(record) {
            case .restoreSuccessor(let key):
                if snapshot.operations[key] != nil {
                    continue
                }
                fallthrough
            case .abortReleaseQuotaAndDeleteProvisional:
                try await abortReplacement(record, snapshot: snapshot)
                return true
            case .garbageCollect:
                _ = try await durability.commit(
                    .init(
                        expectedRevision: snapshot.revision,
                        label: "garbage collect settled replacement",
                        mutations: [.removeReplacement(record.id)]
                    )
                )
                return true
            }
        }
        return false
    }

    /// Aborting a provisional replacement and condemning its file is one
    /// terminal transaction. If the provisional asset is still represented by
    /// a live operation owner, that owner and Entry membership must be settled
    /// in the same commit; otherwise the cleanup tombstone would violate the
    /// store's final-state invariant and brick every launch.
    private func abortReplacement(
        _ record: GaryxReplacementRecord,
        snapshot: GaryxComposerDurabilitySnapshot
    ) async throws {
        let assetID = record.stagedAssetID
        let stagedOwner = snapshot.stagedAssetOwners[assetID]
        let cleanupOwner = snapshot.pendingFileCleanup[assetID]
            ?? Self.replacementCleanupOwner(record, snapshot: snapshot)
        var next = record
        if next.phase == .pendingReplacement { next.abort() }
        next.settle()
        var mutations: [GaryxComposerDurabilityMutation] = [.upsertReplacement(next)]

        if snapshot.pendingFileCleanup[assetID] == nil {
            mutations.append(.registerFileCleanup(assetID: assetID, owner: cleanupOwner))
        }
        if stagedOwner != nil {
            mutations.append(.releaseStagedAsset(assetID))
        }
        if let stagedOwner,
           let original = snapshot.operations[stagedOwner],
           original.stagedAssetID == assetID {
            guard var entry = snapshot.payloadStore.entry(
                stagedOwner.entryID,
                scope: stagedOwner.scope
            ) else {
                throw GaryxComposerDurabilityRecoveryError.operationEntryMissing(stagedOwner)
            }
            var settled = original
            settled.settleIdentityDiscard()
            entry.removeOperation(stagedOwner)
            mutations.append(.upsertOperation(settled))
            mutations.append(.removeManifest(stagedOwner))
            mutations.append(.removeOperation(stagedOwner))
            mutations.append(.upsertEntry(entry))
        }
        mutations.append(.removeReplacement(record.id))
        _ = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "abort replacement and settle provisional owner atomically",
                mutations: mutations
            )
        )
    }

    private static func replacementCleanupOwner(
        _ record: GaryxReplacementRecord,
        snapshot: GaryxComposerDurabilitySnapshot
    ) -> GaryxOperationCapabilityKey {
        if let owner = snapshot.stagedAssetOwners[record.stagedAssetID] {
            return owner
        }
        if let newKey = record.newKey, snapshot.operations[newKey] == nil {
            return newKey
        }
        if snapshot.operations[record.oldKey] == nil {
            return record.oldKey
        }
        var discriminator = 0
        while true {
            let candidate = GaryxOperationCapabilityKey(
                scope: record.oldKey.scope,
                entryID: record.oldKey.entryID,
                generation: record.oldKey.generation,
                reservationID: record.oldKey.reservationID,
                branch: record.oldKey.branch,
                operationID: GaryxOperationID(
                    rawValue: "replacement-cleanup-\(record.id.rawValue)-\(discriminator)"
                )
            )
            if snapshot.operations[candidate] == nil { return candidate }
            discriminator += 1
        }
    }

    private func recoverOneDiscardStep() async throws -> Bool {
        let snapshot = try await durability.load()
        guard let original = snapshot.discardConvergence.values.sorted(by: {
            $0.lifecycle.token.entryID.rawValue < $1.lifecycle.token.entryID.rawValue
        }).first else {
            return false
        }
        let entryID = original.lifecycle.token.entryID
        var convergence = original

        let authoritativeDeliveryRequiresSettlement = snapshot.deliveries.values.contains {
            $0.entryID == entryID
                && $0.scope == convergence.barrier.scope
                && !$0.phase.isSettledForIdentityDiscard
        }
        if !convergence.deliveriesSettled || authoritativeDeliveryRequiresSettlement {
            let settledDeliveries = convergence.settleDeliveries(
                authoritativeRecords: snapshot.deliveries
            )
            var mutations: [GaryxComposerDurabilityMutation] = [
                .upsertDiscardConvergence(convergence),
            ]
            mutations.append(contentsOf: settledDeliveries.map {
                .upsertDelivery($0)
            })
            _ = try await durability.commit(
                .init(
                    expectedRevision: snapshot.revision,
                    label: "discard convergence delivery CAS",
                    mutations: mutations
                )
            )
            return true
        }

        if !convergence.reservationSettled {
            convergence.settleReservation()
            var mutations: [GaryxComposerDurabilityMutation] = [
                .upsertDiscardConvergence(convergence),
            ]
            if let reservationID = original.barrier.reservationID {
                let ledgerKey = GaryxReservationLedgerKey(
                    scope: original.barrier.scope,
                    entryID: entryID,
                    reservationID: reservationID
                )
                if var ledger = snapshot.ledgers[ledgerKey], ledger.terminalOutcome == nil {
                    let generation = try await durability.allocatePayloadGeneration()
                    let refreshed = try await durability.load()
                    guard ledger.settle(.revoked, targetGeneration: generation) else {
                        throw GaryxComposerDurabilityRecoveryError.syntheticReservationCannotConverge(
                            ledgerKey
                        )
                    }
                    mutations.insert(.claimGeneration(generation), at: 0)
                    mutations.insert(.upsertLedger(ledger), at: 0)
                    _ = try await durability.commit(
                        .init(
                            expectedRevision: refreshed.revision,
                            label: "discard convergence force reservation revoke",
                            mutations: mutations
                        )
                    )
                    return true
                }
            }
            _ = try await durability.commit(
                .init(
                    expectedRevision: snapshot.revision,
                    label: "discard convergence clear sealed reservation",
                    mutations: mutations
                )
            )
            return true
        }

        if !convergence.descendantsEmpty {
            convergence.settleSessions()
            _ = try await durability.commit(
                .init(
                    expectedRevision: snapshot.revision,
                    label: "discard convergence session tombstones",
                    mutations: [.upsertDiscardConvergence(convergence)]
                )
            )
            return true
        }

        if !convergence.resourcesSettled {
            convergence.settleResources()
            var mutations: [GaryxComposerDurabilityMutation] = [
                .upsertDiscardConvergence(convergence),
            ]
            if let destination = snapshot.payloadStore
                .entry(entryID, scope: convergence.barrier.scope)?.destination {
                var aliases = snapshot.aliases
                _ = aliases.retireLineage(
                    releasing: convergence.aliasReleases,
                    endingAt: destination,
                    scope: convergence.barrier.scope
                )
                if aliases != snapshot.aliases {
                    mutations.append(.replaceAliases(aliases))
                }
            }
            for (key, operation) in snapshot.operations where
                key.entryID == entryID && key.scope == convergence.barrier.scope {
                if let assetID = operation.stagedAssetID {
                    if snapshot.pendingFileCleanup[assetID] == nil {
                        mutations.append(.registerFileCleanup(assetID: assetID, owner: key))
                    }
                    if snapshot.stagedAssetOwners[assetID] == key {
                        mutations.append(.releaseStagedAsset(assetID))
                    }
                }
                mutations.append(.removeManifest(key))
                mutations.append(.removeOperation(key))
            }
            for record in snapshot.replacements.values where
                record.entryID == entryID && record.scope == convergence.barrier.scope {
                let owner = snapshot.stagedAssetOwners[record.stagedAssetID]
                    ?? record.newKey
                    ?? record.oldKey
                if snapshot.pendingFileCleanup[record.stagedAssetID] == nil {
                    mutations.append(
                        .registerFileCleanup(assetID: record.stagedAssetID, owner: owner)
                    )
                }
                if snapshot.stagedAssetOwners[record.stagedAssetID] != nil {
                    mutations.append(.releaseStagedAsset(record.stagedAssetID))
                }
                mutations.append(.removeReplacement(record.id))
            }
            for feedback in snapshot.feedback.values where
                feedback.entryID == entryID && feedback.scope == convergence.barrier.scope {
                mutations.append(.removeFeedback(feedback.id))
            }
            for lineage in snapshot.attachmentLineages.values where
                lineage.entryID == entryID && lineage.scope == convergence.barrier.scope {
                mutations.append(.removeAttachmentLineage(lineage.id))
            }
            for (key, drained) in snapshot.producerDrained where
                key.token == convergence.lifecycle.token
                    && drained.scope == convergence.barrier.scope {
                mutations.append(.removeProducerDrained(key))
            }
            for (key, close) in snapshot.recoveredInputClosures where
                key.token == convergence.lifecycle.token
                    && close.scope == convergence.barrier.scope {
                mutations.append(.removeRecoveredInputClose(key))
            }
            if snapshot.barriers[entryID] != nil {
                var idleBarrier = convergence.barrier
                idleBarrier.returnToIdle()
                mutations.append(.upsertBarrier(idleBarrier))
                mutations.append(.removeBarrier(entryID))
            }
            mutations.append(
                .removeEntry(scope: convergence.barrier.scope, entryID: entryID)
            )
            _ = try await durability.commit(
                .init(
                    expectedRevision: snapshot.revision,
                    label: "discard convergence resource settlement",
                    mutations: mutations
                )
            )
            return true
        }

        if convergence.lifecycle.phase == .discarding {
            guard convergence.finishToken() else { return false }
            _ = try await durability.commit(
                .init(
                    expectedRevision: snapshot.revision,
                    label: "discard convergence finish token",
                    mutations: [
                        .upsertDiscardConvergence(convergence),
                        .removeEntry(scope: convergence.barrier.scope, entryID: entryID),
                    ]
                )
            )
            return true
        }

        if convergence.persistentTombstoneCount > 0 {
            guard convergence.garbageCollectTombstonesIfEligible() else { return false }
            _ = try await durability.commit(
                .init(
                    expectedRevision: snapshot.revision,
                    label: "discard convergence tombstone GC",
                    mutations: [.upsertDiscardConvergence(convergence)]
                )
            )
            return true
        }

        _ = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "discard convergence record GC",
                mutations: [.removeDiscardConvergence(entryID)]
            )
        )
        return true
    }

    private static func deliveryDisposition(
        _ delivery: GaryxDeliveryRecord,
        snapshot: GaryxComposerDurabilitySnapshot
    ) -> GaryxDurableDeliveryRecoveryDisposition {
        switch delivery.phase {
        case .notDispatched:
            GaryxUndispatchedDeliveryRecoveryPlanner.isOwnedByCreate(
                delivery,
                snapshot: snapshot
            ) ? .userTerminable : .safeToRetry
        case .transportAttempted, .ambiguous:
            .userTerminable
        case .acknowledged:
            .acknowledged
        case .cancelledByDiscard, .evidence, .terminalEvidence,
             .abandoned, .supersededByDuplicate:
            .terminal
        }
    }

    private static func registry(
        for scope: GaryxGatewayScope,
        lifecycle: GaryxGatewayScopeLifecycle
    ) -> GaryxGatewayScopeRegistry {
        var registry = GaryxGatewayScopeRegistry(initialActiveScope: scope)
        switch lifecycle {
        case .active:
            break
        case .suspended:
            _ = registry.suspendActive()
        case .revoked:
            _ = registry.revoke(scope)
        }
        return registry
    }

    private static func recoveryFeedbackID(
        key: GaryxOperationCapabilityKey,
        kind: String
    ) -> GaryxFeedbackID {
        let reservation = key.reservationID.map { String($0.rawValue) } ?? "none"
        let components = [
            "launch",
            kind,
            key.scope.identity,
            String(key.scope.epoch),
            key.entryID.rawValue,
            String(key.generation),
            reservation,
            key.branch.rawValue,
            key.operationID.rawValue,
        ]
        return GaryxFeedbackID(
            rawValue: components.map { "\($0.utf8.count):\($0)" }.joined(separator: "|")
        )
    }

    private static func ledgerSort(
        _ lhs: GaryxProvisionalReservationLedger,
        _ rhs: GaryxProvisionalReservationLedger
    ) -> Bool {
        if lhs.key.scope.identity != rhs.key.scope.identity {
            return lhs.key.scope.identity < rhs.key.scope.identity
        }
        if lhs.key.scope.epoch != rhs.key.scope.epoch {
            return lhs.key.scope.epoch < rhs.key.scope.epoch
        }
        if lhs.key.entryID != rhs.key.entryID {
            return lhs.key.entryID.rawValue < rhs.key.entryID.rawValue
        }
        return lhs.key.reservationID.rawValue < rhs.key.reservationID.rawValue
    }

    private static func operationKeySort(
        _ lhs: GaryxOperationCapabilityKey,
        _ rhs: GaryxOperationCapabilityKey
    ) -> Bool {
        if lhs.scope.identity != rhs.scope.identity {
            return lhs.scope.identity < rhs.scope.identity
        }
        if lhs.scope.epoch != rhs.scope.epoch { return lhs.scope.epoch < rhs.scope.epoch }
        if lhs.entryID != rhs.entryID { return lhs.entryID.rawValue < rhs.entryID.rawValue }
        return lhs.operationID.rawValue < rhs.operationID.rawValue
    }
}

public struct GaryxPreparedDeliveryAttempt: Equatable, Sendable {
    public let deliveryID: GaryxDeliveryRecordID
    public let envelope: GaryxDeliveryEnvelope
    public let durableRevision: UInt64
}

/// The only transport admission gate: callers receive an envelope only after
/// `transportAttempted` has committed. If the process dies afterwards, launch
/// recovery exposes a user-terminable ambiguous record and never auto-retries.
public actor GaryxComposerDeliveryTransportGate {
    private let durability: any GaryxComposerDurabilityStore

    public init(durability: any GaryxComposerDurabilityStore) {
        self.durability = durability
    }

    public func prepareAttempt(
        deliveryID: GaryxDeliveryRecordID
    ) async throws -> GaryxPreparedDeliveryAttempt? {
        let snapshot = try await durability.load()
        guard var delivery = snapshot.deliveries[deliveryID],
              let envelope = delivery.envelope,
              delivery.markTransportAttempted() else {
            return nil
        }
        let committed = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "persist transportAttempted before network",
                mutations: [.upsertDelivery(delivery)]
            )
        )
        return GaryxPreparedDeliveryAttempt(
            deliveryID: deliveryID,
            envelope: envelope,
            durableRevision: committed.revision
        )
    }

    public func performAttempt(
        deliveryID: GaryxDeliveryRecordID,
        network: @Sendable (GaryxDeliveryEnvelope) async throws -> Void
    ) async throws {
        guard let attempt = try await prepareAttempt(deliveryID: deliveryID) else { return }
        do {
            try await network(attempt.envelope)
        } catch {
            try await recordAmbiguous(deliveryID: deliveryID)
            throw error
        }
    }

    public func recordAmbiguous(deliveryID: GaryxDeliveryRecordID) async throws {
        let snapshot = try await durability.load()
        guard var delivery = snapshot.deliveries[deliveryID], delivery.markAmbiguous() else {
            return
        }
        _ = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "persist ambiguous delivery",
                mutations: [.upsertDelivery(delivery)]
            )
        )
    }

    public func acknowledge(deliveryID: GaryxDeliveryRecordID) async throws {
        let snapshot = try await durability.load()
        guard var delivery = snapshot.deliveries[deliveryID] else { return }
        delivery.recordServerAcknowledgement()
        _ = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "persist delivery acknowledgement",
                mutations: [.upsertDelivery(delivery)]
            )
        )
    }
}
