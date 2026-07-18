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
    public var deliveryDispositions: [
        GaryxDeliveryRecordID: GaryxDurableDeliveryRecoveryDisposition
    ]
    public var retryableOperationKeys: Set<GaryxOperationCapabilityKey>

    public init(
        syntheticReservationRecoveries: Int = 0,
        operationSettlements: Int = 0,
        replacementSettlements: Int = 0,
        discardSettlements: Int = 0,
        deliveryDispositions: [
            GaryxDeliveryRecordID: GaryxDurableDeliveryRecoveryDisposition
        ] = [:],
        retryableOperationKeys: Set<GaryxOperationCapabilityKey> = []
    ) {
        self.syntheticReservationRecoveries = syntheticReservationRecoveries
        self.operationSettlements = operationSettlements
        self.replacementSettlements = replacementSettlements
        self.discardSettlements = discardSettlements
        self.deliveryDispositions = deliveryDispositions
        self.retryableOperationKeys = retryableOperationKeys
    }
}

public enum GaryxComposerDurabilityRecoveryError: Error, Equatable, Sendable {
    case syntheticReservationCannotConverge(GaryxReservationLedgerKey)
    case operationEntryMissing(GaryxOperationCapabilityKey)
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
                    (id, Self.deliveryDisposition(delivery))
                }
            )
            report.retryableOperationKeys = Set(snapshot.operations.compactMap { key, operation in
                operation.state == .failedRetryable ? key : nil
            })
            return report
        }
        throw GaryxComposerDurabilityRecoveryError.recoveryDidNotConverge
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
            guard let operation = snapshot.operations[key] else {
                // A manifest without its capability cannot own a network
                // action; archive it and its file before continuing.
                var mutations: [GaryxComposerDurabilityMutation] = [.removeManifest(key)]
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
            let lifecycle = scopes.lifecycle(of: key.scope)
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
                    snapshot: snapshot,
                    eraseEntry: lifecycle == .revoked
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
        snapshot: GaryxComposerDurabilitySnapshot,
        eraseEntry: Bool
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
        if eraseEntry {
            mutations.append(.removeEntry(scope: key.scope, entryID: key.entryID))
        } else {
            mutations.append(.upsertEntry(entry))
        }
        _ = try await durability.commit(
            .init(
                expectedRevision: snapshot.revision,
                label: "settle operation recovery",
                mutations: mutations
            )
        )
    }

    private func recoverOneReplacement() async throws -> Bool {
        let snapshot = try await durability.load()
        for record in snapshot.replacements.values.sorted(by: { $0.id.rawValue < $1.id.rawValue }) {
            let lifecycle = scopes.lifecycle(of: record.scope)
            switch GaryxReplacementPlanner.recover(record) {
            case .restoreSuccessor(let key):
                if lifecycle != .revoked, snapshot.operations[key] != nil {
                    continue
                }
                fallthrough
            case .abortReleaseQuotaAndDeleteProvisional:
                var next = record
                if next.phase == .pendingReplacement { next.abort() }
                var mutations: [GaryxComposerDurabilityMutation] = [.upsertReplacement(next)]
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
                next.settle()
                mutations.append(.upsertReplacement(next))
                mutations.append(.removeReplacement(record.id))
                _ = try await durability.commit(
                    .init(
                        expectedRevision: snapshot.revision,
                        label: "abort replacement and retain physical cleanup proof",
                        mutations: mutations
                    )
                )
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

    private func recoverOneDiscardStep() async throws -> Bool {
        let snapshot = try await durability.load()
        guard let original = snapshot.discardConvergence.values.sorted(by: {
            $0.lifecycle.token.entryID.rawValue < $1.lifecycle.token.entryID.rawValue
        }).first else {
            return false
        }
        let entryID = original.lifecycle.token.entryID
        var convergence = original

        if !convergence.deliveriesSettled {
            convergence.settleDeliveries()
            var mutations: [GaryxComposerDurabilityMutation] = [
                .upsertDiscardConvergence(convergence),
            ]
            mutations.append(contentsOf: convergence.deliveries.values.map {
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
                let retiringSources = aliases.partitions[convergence.barrier.scope, default: [:]]
                    .values
                    .filter { $0.source == destination || $0.target == destination }
                    .map(\.source)
                for source in retiringSources {
                    _ = aliases.markDrained(
                        source: source,
                        scope: convergence.barrier.scope
                    )
                }
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
        _ delivery: GaryxDeliveryRecord
    ) -> GaryxDurableDeliveryRecoveryDisposition {
        switch delivery.phase {
        case .notDispatched:
            .safeToRetry
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
        GaryxFeedbackID(
            rawValue: "launch-\(kind)-\(key.scope.identity)-\(key.scope.epoch)-\(key.operationID.rawValue)"
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
