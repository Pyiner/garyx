import Foundation

public struct GaryxDurableProducerDrainedRecord: Equatable, Codable, Sendable {
    public let scope: GaryxGatewayScope
    public let entryID: GaryxComposerPayloadEntryID
    public let reservationID: GaryxSendReservationID?
    public let record: GaryxProducerDrainedRecord

    public init(
        scope: GaryxGatewayScope,
        entryID: GaryxComposerPayloadEntryID,
        reservationID: GaryxSendReservationID?,
        record: GaryxProducerDrainedRecord
    ) {
        self.scope = scope
        self.entryID = entryID
        self.reservationID = reservationID
        self.record = record
    }
}

public struct GaryxRecoveredInputCloseRecord: Equatable, Codable, Sendable {
    public let key: GaryxSessionDescendantKey
    public let scope: GaryxGatewayScope
    public let entryID: GaryxComposerPayloadEntryID
    public let reservationID: GaryxSendReservationID
    public let targetGeneration: UInt64
    public let finalSequence: UInt64
    public let finalText: String
    public let nextEpoch: UInt64
    public let closePublicationCount: Int

    public init(
        key: GaryxSessionDescendantKey,
        scope: GaryxGatewayScope,
        entryID: GaryxComposerPayloadEntryID,
        reservationID: GaryxSendReservationID,
        targetGeneration: UInt64,
        finalSequence: UInt64,
        finalText: String
    ) {
        self.key = key
        self.scope = scope
        self.entryID = entryID
        self.reservationID = reservationID
        self.targetGeneration = targetGeneration
        self.finalSequence = finalSequence
        self.finalText = finalText
        nextEpoch = key.epoch + 1
        closePublicationCount = 1
    }
}

/// Complete A3 model snapshot. A4d-1 will map these records to concrete DB
/// rows; A3 deliberately provides no filesystem or database implementation.
public struct GaryxComposerDurabilitySnapshot: Equatable, Codable, Sendable {
    public fileprivate(set) var revision: UInt64
    public fileprivate(set) var payloadStore: GaryxComposerPayloadStore
    public fileprivate(set) var aliases: GaryxComposerAliasTable
    public fileprivate(set) var operations: [GaryxOperationCapabilityKey: GaryxOperationCapability]
    public fileprivate(set) var manifests: [GaryxOperationCapabilityKey: GaryxOperationManifest]
    public fileprivate(set) var replacements: [GaryxReplacementID: GaryxReplacementRecord]
    public fileprivate(set) var conflicts: [GaryxPayloadConflictSetID: GaryxPayloadConflictSet]
    public fileprivate(set) var feedback: [GaryxFeedbackID: GaryxOperationFeedback]
    public fileprivate(set) var attachmentLineages: [
        GaryxAttachmentLineageID: GaryxAttachmentLineageTombstone
    ]
    public fileprivate(set) var barriers: [GaryxComposerPayloadEntryID: GaryxSendCommitBarrier]
    public fileprivate(set) var ledgers: [GaryxReservationLedgerKey: GaryxProvisionalReservationLedger]
    public fileprivate(set) var producerDrained: [GaryxSessionDescendantKey: GaryxDurableProducerDrainedRecord]
    public fileprivate(set) var recoveredInputClosures: [
        GaryxSessionDescendantKey: GaryxRecoveredInputCloseRecord
    ]
    public fileprivate(set) var deliveries: [GaryxDeliveryRecordID: GaryxDeliveryRecord]
    public fileprivate(set) var discardConvergence: [GaryxComposerPayloadEntryID: GaryxPayloadDiscardConvergence]
    public fileprivate(set) var createDeliveries: [GaryxCreateDeliveryKey: GaryxCreateDeliveryState]
    public fileprivate(set) var scopeRegistry: GaryxGatewayScopeRegistry
    public fileprivate(set) var stagedAssetOwners: [GaryxStagedAssetID: GaryxOperationCapabilityKey]
    public fileprivate(set) var stagedAssetReservedBytes: [GaryxStagedAssetID: Int]
    /// Durable condemned-file tombstones. An entry may outlive its former
    /// operation owner and is removed only after physical deletion succeeds.
    public fileprivate(set) var pendingFileCleanup: [
        GaryxStagedAssetID: GaryxOperationCapabilityKey
    ]
    public fileprivate(set) var reservedBytes: Int
    public fileprivate(set) var generationHighWatermark: UInt64
    public fileprivate(set) var reservationHighWatermark: UInt64
    /// Every generation at or below this floor is permanently consumed or
    /// abandoned. Exact claims are retained only for the current hi-lo block.
    public fileprivate(set) var generationClaimFloor: UInt64
    public fileprivate(set) var claimedGenerations: Set<UInt64>
    public fileprivate(set) var tombstoneBudget: GaryxPersistentTombstoneBudget

    public init(
        revision: UInt64 = 0,
        payloadStore: GaryxComposerPayloadStore = .init(),
        aliases: GaryxComposerAliasTable = .init(),
        operations: [GaryxOperationCapabilityKey: GaryxOperationCapability] = [:],
        manifests: [GaryxOperationCapabilityKey: GaryxOperationManifest] = [:],
        replacements: [GaryxReplacementID: GaryxReplacementRecord] = [:],
        conflicts: [GaryxPayloadConflictSetID: GaryxPayloadConflictSet] = [:],
        feedback: [GaryxFeedbackID: GaryxOperationFeedback] = [:],
        attachmentLineages: [
            GaryxAttachmentLineageID: GaryxAttachmentLineageTombstone
        ] = [:],
        barriers: [GaryxComposerPayloadEntryID: GaryxSendCommitBarrier] = [:],
        ledgers: [GaryxReservationLedgerKey: GaryxProvisionalReservationLedger] = [:],
        producerDrained: [GaryxSessionDescendantKey: GaryxDurableProducerDrainedRecord] = [:],
        recoveredInputClosures: [
            GaryxSessionDescendantKey: GaryxRecoveredInputCloseRecord
        ] = [:],
        deliveries: [GaryxDeliveryRecordID: GaryxDeliveryRecord] = [:],
        discardConvergence: [GaryxComposerPayloadEntryID: GaryxPayloadDiscardConvergence] = [:],
        createDeliveries: [GaryxCreateDeliveryKey: GaryxCreateDeliveryState] = [:],
        scopeRegistry: GaryxGatewayScopeRegistry = .init(),
        stagedAssetOwners: [GaryxStagedAssetID: GaryxOperationCapabilityKey] = [:],
        stagedAssetReservedBytes: [GaryxStagedAssetID: Int] = [:],
        pendingFileCleanup: [GaryxStagedAssetID: GaryxOperationCapabilityKey] = [:],
        reservedBytes: Int = 0,
        generationHighWatermark: UInt64 = 0,
        reservationHighWatermark: UInt64 = 0,
        generationClaimFloor: UInt64 = 0,
        claimedGenerations: Set<UInt64> = [],
        tombstoneBudget: GaryxPersistentTombstoneBudget = .init()
    ) {
        precondition(reservedBytes >= 0 && generationClaimFloor <= generationHighWatermark)
        self.revision = revision
        self.payloadStore = payloadStore
        self.aliases = aliases
        self.operations = operations
        self.manifests = manifests
        self.replacements = replacements
        self.conflicts = conflicts
        self.feedback = feedback
        self.attachmentLineages = attachmentLineages
        self.barriers = barriers
        self.ledgers = ledgers
        self.producerDrained = producerDrained
        self.recoveredInputClosures = recoveredInputClosures
        self.deliveries = deliveries
        self.discardConvergence = discardConvergence
        self.createDeliveries = createDeliveries
        self.scopeRegistry = scopeRegistry
        self.stagedAssetOwners = stagedAssetOwners
        self.stagedAssetReservedBytes = stagedAssetReservedBytes
        self.pendingFileCleanup = pendingFileCleanup
        self.reservedBytes = reservedBytes
        self.generationHighWatermark = generationHighWatermark
        self.reservationHighWatermark = reservationHighWatermark
        self.generationClaimFloor = generationClaimFloor
        self.claimedGenerations = claimedGenerations
        self.tombstoneBudget = tombstoneBudget
    }

    public var persistentTombstoneUsage: GaryxPersistentTombstoneUsage {
        let correlationBytes = deliveries.values.compactMap(\.persistentTombstoneEstimatedBytes)
        let createCorrelationBytes = createDeliveries.values.compactMap(
            \.persistentTombstoneEstimatedBytes
        )
        let discardCounts = discardConvergence.values.map(\.persistentTombstoneCount)
        let discardBytes = discardConvergence.values.map(\.persistentTombstoneBytes)
        return GaryxPersistentTombstoneUsage(
            correlationCount: correlationBytes.count,
            correlationBytes: correlationBytes.reduce(0, +),
            createCorrelationCount: createCorrelationBytes.count,
            createCorrelationBytes: createCorrelationBytes.reduce(0, +),
            discardFinalizationCount: discardCounts.reduce(0, +),
            discardFinalizationBytes: discardBytes.reduce(0, +)
        )
    }
}

public enum GaryxComposerDurabilityMutation: Equatable, Sendable {
    case upsertEntry(GaryxComposerPayloadEntry)
    case removeEntry(scope: GaryxGatewayScope, entryID: GaryxComposerPayloadEntryID)
    case replaceAliases(GaryxComposerAliasTable)
    case upsertOperation(GaryxOperationCapability)
    case removeOperation(GaryxOperationCapabilityKey)
    case upsertManifest(GaryxOperationManifest)
    case removeManifest(GaryxOperationCapabilityKey)
    case upsertReplacement(GaryxReplacementRecord)
    case removeReplacement(GaryxReplacementID)
    case upsertConflict(GaryxPayloadConflictSet)
    case removeConflict(GaryxPayloadConflictSetID)
    case upsertFeedback(GaryxOperationFeedback)
    case removeFeedback(GaryxFeedbackID)
    case upsertAttachmentLineage(GaryxAttachmentLineageTombstone)
    case removeAttachmentLineage(GaryxAttachmentLineageID)
    case upsertBarrier(GaryxSendCommitBarrier)
    case removeBarrier(GaryxComposerPayloadEntryID)
    case upsertLedger(GaryxProvisionalReservationLedger)
    case removeLedger(GaryxReservationLedgerKey)
    case synthesizeReservationRevocation(GaryxReservationLedgerKey)
    case persistReservationTargetMapping(GaryxReservationLedgerKey, generation: UInt64)
    case upsertProducerDrained(GaryxSessionDescendantKey, GaryxDurableProducerDrainedRecord)
    case removeProducerDrained(GaryxSessionDescendantKey)
    case upsertRecoveredInputClose(GaryxRecoveredInputCloseRecord)
    case removeRecoveredInputClose(GaryxSessionDescendantKey)
    case upsertDelivery(GaryxDeliveryRecord)
    case removeDelivery(GaryxDeliveryRecordID)
    case upsertDiscardConvergence(GaryxPayloadDiscardConvergence)
    case removeDiscardConvergence(GaryxComposerPayloadEntryID)
    case upsertCreateDelivery(GaryxCreateDeliveryState)
    case removeCreateDelivery(GaryxCreateDeliveryKey)
    case replaceScopeRegistry(GaryxGatewayScopeRegistry)
    case reserveStagedAsset(
        assetID: GaryxStagedAssetID,
        owner: GaryxOperationCapabilityKey,
        bytes: Int
    )
    case releaseStagedAsset(GaryxStagedAssetID)
    case registerFileCleanup(assetID: GaryxStagedAssetID, owner: GaryxOperationCapabilityKey)
    case completeFileCleanup(GaryxStagedAssetID)
    case setGenerationHighWatermark(UInt64)
    case claimGeneration(UInt64)
    case setReservationHighWatermark(UInt64)
}

public struct GaryxComposerDurabilityTransaction: Equatable, Sendable {
    public let expectedRevision: UInt64
    public let label: String
    public let mutations: [GaryxComposerDurabilityMutation]

    public init(
        expectedRevision: UInt64,
        label: String,
        mutations: [GaryxComposerDurabilityMutation]
    ) {
        self.expectedRevision = expectedRevision
        self.label = label
        self.mutations = mutations
    }
}

public struct GaryxReplacementFeedbackSwapPlan: Equatable, Sendable {
    public let oldOperation: GaryxOperationCapability
    public let successorOperation: GaryxOperationCapability
    public let replacement: GaryxReplacementRecord
    public let feedback: GaryxOperationFeedback
    public let successorManifest: GaryxOperationManifest
    public let transaction: GaryxComposerDurabilityTransaction

    public init(
        oldOperation: GaryxOperationCapability,
        successorOperation: GaryxOperationCapability,
        replacement: GaryxReplacementRecord,
        feedback: GaryxOperationFeedback,
        successorManifest: GaryxOperationManifest,
        transaction: GaryxComposerDurabilityTransaction
    ) {
        self.oldOperation = oldOperation
        self.successorOperation = successorOperation
        self.replacement = replacement
        self.feedback = feedback
        self.successorManifest = successorManifest
        self.transaction = transaction
    }
}

/// Plans the retry swap from the authoritative durable snapshot. This keeps
/// the interactive conversation retry path from inventing an owner map: O1's
/// live manifest, Entry membership, staged-file owner, and quota reservation
/// must all agree before one transaction can transfer them to O2.
public enum GaryxReplacementFeedbackSwapPlanner {
    public static func plan(
        snapshot: GaryxComposerDurabilitySnapshot,
        successor: GaryxOperationCapability,
        replacementID: GaryxReplacementID,
        feedbackID: GaryxFeedbackID,
        scopes: GaryxGatewayScopeRegistry,
        beginUpload: Bool = false
    ) -> GaryxReplacementFeedbackSwapPlan? {
        guard let replacement = snapshot.replacements[replacementID] else { return nil }
        return makePlan(
            snapshot: snapshot,
            successor: successor,
            replacement: replacement,
            feedbackID: feedbackID,
            scopes: scopes,
            beginUpload: beginUpload
        )
    }

    /// Starts an explicit retry from the retained staged asset. Journal
    /// admission, owner transfer, feedback acknowledgement, and (optionally)
    /// the new upload-attempt boundary publish in one transaction.
    public static func planRetry(
        snapshot: GaryxComposerDurabilitySnapshot,
        oldOperationKey: GaryxOperationCapabilityKey,
        successor: GaryxOperationCapability,
        replacementID: GaryxReplacementID,
        feedbackID: GaryxFeedbackID,
        scopes: GaryxGatewayScopeRegistry,
        beginUpload: Bool = true
    ) -> GaryxReplacementFeedbackSwapPlan? {
        guard let old = snapshot.operations[oldOperationKey],
              old.state == .failedRetryable,
              let stagedAssetID = old.stagedAssetID else {
            return nil
        }
        let replacement = GaryxReplacementRecord(
            id: replacementID,
            scope: oldOperationKey.scope,
            entryID: oldOperationKey.entryID,
            oldKey: oldOperationKey,
            reservationID: oldOperationKey.reservationID,
            branch: oldOperationKey.branch,
            stagedAssetID: stagedAssetID,
            reservedBytes: old.reservedBytes
        )
        return makePlan(
            snapshot: snapshot,
            successor: successor,
            replacement: replacement,
            feedbackID: feedbackID,
            scopes: scopes,
            beginUpload: beginUpload
        )
    }

    private static func makePlan(
        snapshot: GaryxComposerDurabilitySnapshot,
        successor: GaryxOperationCapability,
        replacement: GaryxReplacementRecord,
        feedbackID: GaryxFeedbackID,
        scopes: GaryxGatewayScopeRegistry,
        beginUpload: Bool
    ) -> GaryxReplacementFeedbackSwapPlan? {
        var replacement = replacement
        guard var old = snapshot.operations[replacement.oldKey],
              var feedback = snapshot.feedback[feedbackID],
              var entry = snapshot.payloadStore.entry(
                  replacement.entryID,
                  scope: replacement.scope
              ),
              entry.operationKeys.contains(replacement.oldKey),
              let oldManifest = snapshot.manifests[replacement.oldKey],
              old.stagedAssetID == replacement.stagedAssetID,
              old.reservedBytes == replacement.reservedBytes,
              snapshot.stagedAssetOwners[replacement.stagedAssetID] == replacement.oldKey,
              snapshot.stagedAssetReservedBytes[replacement.stagedAssetID]
                  == replacement.reservedBytes else {
            return nil
        }
        var successor = successor
        guard GaryxReplacementFeedbackSwapReducer.commit(
            old: &old,
            successor: &successor,
            record: &replacement,
            feedback: &feedback,
            lifecycle: entry.lifecycle.snapshot,
            scopes: scopes
        ) == .committed else {
            return nil
        }

        entry.addOperation(successor.context.key)
        if beginUpload {
            guard successor.transition(
                expectedKey: successor.context.key,
                to: .uploading,
                lifecycle: entry.lifecycle.snapshot,
                scopes: scopes
            ) == .applied,
            successor.markUploadAttempted(
                expectedKey: successor.context.key,
                authoritativeEntry: entry,
                lifecycle: entry.lifecycle.snapshot,
                scopes: scopes
            ) == .applied else {
                return nil
            }
        }
        let successorManifest = GaryxOperationManifest(
            key: successor.context.key,
            stagedPath: oldManifest.stagedPath,
            state: successor.state,
            uploadAttempted: successor.uploadAttempted
        )
        let transaction = GaryxComposerDurabilityTransaction(
            expectedRevision: snapshot.revision,
            label: "transfer retryable staged payload to replacement operation",
            mutations: [
                .upsertEntry(entry),
                .upsertOperation(old),
                .removeManifest(replacement.oldKey),
                .upsertOperation(successor),
                .upsertManifest(successorManifest),
                .upsertReplacement(replacement),
                .upsertFeedback(feedback),
                .releaseStagedAsset(replacement.stagedAssetID),
                .reserveStagedAsset(
                    assetID: replacement.stagedAssetID,
                    owner: successor.context.key,
                    bytes: replacement.reservedBytes
                ),
            ]
        )
        return GaryxReplacementFeedbackSwapPlan(
            oldOperation: old,
            successorOperation: successor,
            replacement: replacement,
            feedback: feedback,
            successorManifest: successorManifest,
            transaction: transaction
        )
    }
}

public struct GaryxSyntheticReservationRecoveryPlan: Equatable, Sendable {
    public let ledgerKey: GaryxReservationLedgerKey
    public let mergeGeneration: UInt64
    public let performedSteps: [GaryxSyntheticRecoveryStep]
    public let transaction: GaryxComposerDurabilityTransaction

    public init(
        ledgerKey: GaryxReservationLedgerKey,
        mergeGeneration: UInt64,
        performedSteps: [GaryxSyntheticRecoveryStep],
        transaction: GaryxComposerDurabilityTransaction
    ) {
        self.ledgerKey = ledgerKey
        self.mergeGeneration = mergeGeneration
        self.performedSteps = performedSteps
        self.transaction = transaction
    }
}

/// Builds the five-step startup revocation as one durability transaction.
/// Every reported step contributes concrete mutations; the fake store can kill
/// at any mutation boundary without publishing a partial outcome.
public enum GaryxSyntheticReservationRecoveryPlanner {
    public static func plan(
        snapshot: GaryxComposerDurabilitySnapshot,
        ledgerKey: GaryxReservationLedgerKey,
        mergeGeneration: UInt64,
        conflictSetID: GaryxPayloadConflictSetID? = nil
    ) -> GaryxSyntheticReservationRecoveryPlan? {
        guard let ledger = snapshot.ledgers[ledgerKey],
              ledger.terminalOutcome == nil,
              ledger.targetMapping == nil,
              mergeGeneration > ledger.followupGeneration,
              mergeGeneration <= snapshot.generationHighWatermark,
              mergeGeneration > snapshot.generationClaimFloor,
              !snapshot.claimedGenerations.contains(mergeGeneration),
              var entry = snapshot.payloadStore.entry(ledgerKey.entryID, scope: ledgerKey.scope),
              var barrier = snapshot.barriers[ledgerKey.entryID],
              barrier.scope == ledgerKey.scope,
              barrier.reservationID == ledgerKey.reservationID,
              barrier.phase == .sealed,
              barrier.envelopeGeneration == ledger.envelopeGeneration,
              barrier.followupGeneration == ledger.followupGeneration else {
            return nil
        }

        let drained = snapshot.producerDrained
            .filter { _, value in
                value.scope == ledgerKey.scope
                    && value.entryID == ledgerKey.entryID
                    && value.reservationID == ledgerKey.reservationID
            }
            .sorted { lhs, rhs in
                if lhs.key.epoch != rhs.key.epoch { return lhs.key.epoch < rhs.key.epoch }
                return lhs.key.sessionID.rawValue < rhs.key.sessionID.rawValue
            }
        let followupText = drained.last?.value.record.bufferedText
            ?? barrier.provisionalFollowupText
        let mergedText = (barrier.envelopeText ?? "") + followupText
        guard barrier.revoke(
            mergeGeneration: mergeGeneration,
            lifecycle: entry.lifecycle.snapshot
        ) != nil else {
            return nil
        }
        entry.recoverSyntheticRevocation(
            envelopeGeneration: ledger.envelopeGeneration,
            followupGeneration: ledger.followupGeneration,
            mergeGeneration: mergeGeneration,
            mergedText: mergedText
        )

        var stepThreeMutations: [GaryxComposerDurabilityMutation] = []
        let collisions = snapshot.payloadStore.entriesByScope[ledgerKey.scope]?.values
            .filter { $0.id != entry.id && $0.destination == entry.destination }
            .map(\.id)
            .sorted { $0.rawValue < $1.rawValue } ?? []
        if !collisions.isEmpty {
            guard let conflictSetID else { return nil }
            var conflict = snapshot.conflicts[conflictSetID]
                ?? GaryxPayloadConflictSet(id: conflictSetID, scope: ledgerKey.scope)
            guard conflict.scope == ledgerKey.scope else { return nil }
            for candidateID in ([entry.id] + collisions).sorted(by: { $0.rawValue < $1.rawValue }) {
                guard conflict.admitCandidate(
                    .init(entryID: candidateID, label: "synthetic-recovery-\(candidateID.rawValue)"),
                    membershipDurabilityAvailable: true
                ) else {
                    return nil
                }
            }
            stepThreeMutations.append(.upsertConflict(conflict))
        }

        var stepFourMutations: [GaryxComposerDurabilityMutation] = []
        let operationKeys = Set(snapshot.operations.keys).union(snapshot.manifests.keys)
            .filter {
                $0.scope == ledgerKey.scope
                    && $0.entryID == ledgerKey.entryID
                    && $0.reservationID == ledgerKey.reservationID
                    && $0.generation == ledger.followupGeneration
            }
            .sorted { $0.operationID.rawValue < $1.operationID.rawValue }
        for oldKey in operationKeys {
            let newKey = oldKey.remapped(toGeneration: mergeGeneration)
            entry.remapOperationKey(from: oldKey, to: newKey)
            if let operation = snapshot.operations[oldKey] {
                let remapped = operation.remapped(toGeneration: mergeGeneration)
                stepFourMutations.append(.removeOperation(oldKey))
                stepFourMutations.append(.upsertOperation(remapped))
                if let assetID = operation.stagedAssetID,
                   snapshot.stagedAssetOwners[assetID] == oldKey {
                    stepFourMutations.append(.releaseStagedAsset(assetID))
                    stepFourMutations.append(
                        .reserveStagedAsset(
                            assetID: assetID,
                            owner: newKey,
                            bytes: snapshot.stagedAssetReservedBytes[assetID]
                                ?? operation.reservedBytes
                        )
                    )
                }
            }
            if let manifest = snapshot.manifests[oldKey] {
                stepFourMutations.append(.removeManifest(oldKey))
                stepFourMutations.append(
                    .upsertManifest(manifest.remapped(toGeneration: mergeGeneration))
                )
            }
            for replacement in snapshot.replacements.values where
                replacement.oldKey == oldKey || replacement.newKey == oldKey {
                stepFourMutations.append(
                    .upsertReplacement(replacement.remapped(from: oldKey, to: newKey))
                )
            }
        }

        stepThreeMutations.insert(.upsertEntry(entry), at: 0)
        for (key, durable) in drained {
            stepThreeMutations.append(
                .upsertRecoveredInputClose(
                    GaryxRecoveredInputCloseRecord(
                        key: key,
                        scope: ledgerKey.scope,
                        entryID: ledgerKey.entryID,
                        reservationID: ledgerKey.reservationID,
                        targetGeneration: mergeGeneration,
                        finalSequence: durable.record.finalSequence,
                        finalText: mergedText
                    )
                )
            )
            stepThreeMutations.append(.removeProducerDrained(key))
        }
        stepFourMutations.append(.upsertBarrier(barrier))

        let mutations: [GaryxComposerDurabilityMutation] = [
            .synthesizeReservationRevocation(ledgerKey),
            .claimGeneration(mergeGeneration),
        ] + stepThreeMutations + stepFourMutations + [
            .persistReservationTargetMapping(ledgerKey, generation: mergeGeneration),
        ]
        return GaryxSyntheticReservationRecoveryPlan(
            ledgerKey: ledgerKey,
            mergeGeneration: mergeGeneration,
            performedSteps: GaryxSyntheticRecoveryStep.allCases,
            transaction: GaryxComposerDurabilityTransaction(
                expectedRevision: snapshot.revision,
                label: "synthetic reservation revocation",
                mutations: mutations
            )
        )
    }
}

public struct GaryxPayloadGenerationResetPlan: Equatable, Sendable {
    public let entryID: GaryxComposerPayloadEntryID
    public let clearedGeneration: UInt64
    public let allocatedGeneration: UInt64
    public let cancelledOperationKeys: [GaryxOperationCapabilityKey]
    /// Replacement journals retained in `.aborted` until the caller has
    /// deleted their provisional file and released its physical quota.
    public let pendingReplacementCleanupIDs: [GaryxReplacementID]
    /// Condemned staged files whose durable cleanup tombstones survive this
    /// transaction. The caller may remove each tombstone only after its
    /// physical file deletion has completed idempotently.
    public let pendingFileCleanupAssetIDs: [GaryxStagedAssetID]
    public let transaction: GaryxComposerDurabilityTransaction

    public init(
        entryID: GaryxComposerPayloadEntryID,
        clearedGeneration: UInt64,
        allocatedGeneration: UInt64,
        cancelledOperationKeys: [GaryxOperationCapabilityKey],
        pendingReplacementCleanupIDs: [GaryxReplacementID],
        pendingFileCleanupAssetIDs: [GaryxStagedAssetID],
        transaction: GaryxComposerDurabilityTransaction
    ) {
        self.entryID = entryID
        self.clearedGeneration = clearedGeneration
        self.allocatedGeneration = allocatedGeneration
        self.cancelledOperationKeys = cancelledOperationKeys
        self.pendingReplacementCleanupIDs = pendingReplacementCleanupIDs
        self.pendingFileCleanupAssetIDs = pendingFileCleanupAssetIDs
        self.transaction = transaction
    }
}

/// Plans PayloadGenerationReset as one durability transaction. The lightweight
/// identity reducer rejects entries with operation descendants; this planner is
/// the only reset path that may clear them because it cancels capability state,
/// manifest ownership, staged-file quota ownership, feedback references, and
/// the Entry generation before publishing any part of the result. Physical
/// deletion remains represented by a condemned-file tombstone until confirmed.
public enum GaryxPayloadGenerationResetPlanner {
    public static func plan(
        snapshot: GaryxComposerDurabilitySnapshot,
        scope: GaryxGatewayScope,
        entryID: GaryxComposerPayloadEntryID,
        generation: UInt64,
        allocatedGeneration: UInt64,
        producerLive: Bool
    ) -> GaryxPayloadGenerationResetPlan? {
        guard producerLive,
              allocatedGeneration > generation,
              allocatedGeneration <= snapshot.generationHighWatermark,
              allocatedGeneration > snapshot.generationClaimFloor,
              !snapshot.claimedGenerations.contains(allocatedGeneration),
              snapshot.barriers[entryID].map({ $0.phase == .idle }) ?? true,
              var entry = snapshot.payloadStore.entry(entryID, scope: scope),
              entry.lifecycle.phase == .active,
              entry.currentGeneration == generation else {
            return nil
        }

        let affectedKeys = Set(entry.operationKeys)
            .union(snapshot.operations.keys)
            .union(snapshot.manifests.keys)
            .filter {
                $0.scope == scope
                    && $0.entryID == entryID
                    && $0.generation == generation
            }
            .sorted { lhs, rhs in
                lhs.operationID.rawValue < rhs.operationID.rawValue
            }
        let affectedKeySet = Set(affectedKeys)
        let affectedOperationIDs = Set(affectedKeys.map(\.operationID))
        let affectedReplacementIDs = snapshot.replacements.values
            .filter {
                $0.scope == scope
                    && $0.entryID == entryID
                    && (affectedKeySet.contains($0.oldKey)
                        || $0.newKey.map(affectedKeySet.contains) == true)
            }
            .map(\.id)
            .sorted { $0.rawValue < $1.rawValue }

        var mutations: [GaryxComposerDurabilityMutation] = [
            .claimGeneration(allocatedGeneration),
        ]
        for key in affectedKeys {
            if var operation = snapshot.operations[key] {
                operation.settleIdentityDiscard()
                mutations.append(.upsertOperation(operation))
            }
        }

        var affectedAssets = Set(
            snapshot.stagedAssetOwners.compactMap { assetID, owner in
                affectedKeySet.contains(owner) ? assetID : nil
            }
        )
        // A failed-terminal operation may have already released ownership
        // while retaining the canonical pending-file-cleanup obligation. Its
        // own staged asset is therefore an independent discovery source.
        for key in affectedKeys {
            if let assetID = snapshot.operations[key]?.stagedAssetID {
                affectedAssets.insert(assetID)
            }
        }
        for replacementID in affectedReplacementIDs {
            if let replacement = snapshot.replacements[replacementID] {
                affectedAssets.insert(replacement.stagedAssetID)
            }
        }
        var pendingFileCleanupAssetIDs: [GaryxStagedAssetID] = []
        for assetID in affectedAssets.sorted(by: { $0.rawValue < $1.rawValue }) {
            if let owner = snapshot.stagedAssetOwners[assetID] {
                if snapshot.pendingFileCleanup[assetID] == nil {
                    mutations.append(.registerFileCleanup(assetID: assetID, owner: owner))
                }
                mutations.append(.releaseStagedAsset(assetID))
            }
            if snapshot.pendingFileCleanup[assetID] != nil
                || snapshot.stagedAssetOwners[assetID] != nil {
                pendingFileCleanupAssetIDs.append(assetID)
            }
        }

        for key in affectedKeys {
            mutations.append(.removeManifest(key))
            mutations.append(.removeOperation(key))
            entry.removeOperation(key)
        }
        var pendingReplacementCleanupIDs: [GaryxReplacementID] = []
        for replacementID in affectedReplacementIDs {
            guard var replacement = snapshot.replacements[replacementID] else { continue }
            switch replacement.phase {
            case .pendingReplacement:
                replacement.abort()
                mutations.append(.upsertReplacement(replacement))
                pendingReplacementCleanupIDs.append(replacementID)
            case .aborted:
                // `.aborted` is the durable cleanup obligation. Retain it
                // until the file/quota executor settles the record.
                pendingReplacementCleanupIDs.append(replacementID)
            case .committed:
                replacement.settle()
                mutations.append(.upsertReplacement(replacement))
                mutations.append(.removeReplacement(replacementID))
            case .settled:
                mutations.append(.removeReplacement(replacementID))
            }
        }

        let affectedFeedback = snapshot.feedback.values
            .filter {
                $0.scope == scope
                    && $0.entryID == entryID
                    && $0.operationID.map(affectedOperationIDs.contains) == true
            }
            .sorted { $0.id.rawValue < $1.id.rawValue }
        for originalFeedback in affectedFeedback {
            var feedback = originalFeedback
            feedback.archive()
            mutations.append(.upsertFeedback(feedback))
            entry.removeFeedbackReference(feedback.id)
            if let lineageID = feedback.lineageID,
               var lineage = snapshot.attachmentLineages[lineageID],
               lineage.release(after: feedback) {
                mutations.append(.upsertAttachmentLineage(lineage))
            }
        }

        guard entry.resetGeneration(
            generation,
            to: allocatedGeneration,
            barrierIdle: true,
            producerLive: true
        ) else {
            return nil
        }
        mutations.append(.upsertEntry(entry))

        return GaryxPayloadGenerationResetPlan(
            entryID: entryID,
            clearedGeneration: generation,
            allocatedGeneration: allocatedGeneration,
            cancelledOperationKeys: affectedKeys,
            pendingReplacementCleanupIDs: pendingReplacementCleanupIDs,
            pendingFileCleanupAssetIDs: pendingFileCleanupAssetIDs,
            transaction: GaryxComposerDurabilityTransaction(
                expectedRevision: snapshot.revision,
                label: "payload generation reset",
                mutations: mutations
            )
        )
    }
}

public struct GaryxOperationRemovalFeedbackPlan: Equatable, Sendable {
    public let operationKey: GaryxOperationCapabilityKey
    public let feedbackID: GaryxFeedbackID
    /// Condemned staged file retained as a durable cleanup tombstone until
    /// physical deletion is acknowledged in a later transaction.
    public let pendingFileCleanupAssetID: GaryxStagedAssetID?
    public let transaction: GaryxComposerDurabilityTransaction

    public init(
        operationKey: GaryxOperationCapabilityKey,
        feedbackID: GaryxFeedbackID,
        pendingFileCleanupAssetID: GaryxStagedAssetID?,
        transaction: GaryxComposerDurabilityTransaction
    ) {
        self.operationKey = operationKey
        self.feedbackID = feedbackID
        self.pendingFileCleanupAssetID = pendingFileCleanupAssetID
        self.transaction = transaction
    }
}

/// Plans the explicit remove action as one durability transaction. The failure
/// chip is acknowledged only in the same publication that cancels and removes
/// its operation child, records the staged-file deletion obligation, releases
/// staged-file quota ownership, clears the Entry reference, and releases any
/// failed-terminal attachment lineage.
public enum GaryxOperationRemovalFeedbackPlanner {
    public static func plan(
        snapshot: GaryxComposerDurabilitySnapshot,
        operationKey: GaryxOperationCapabilityKey,
        feedbackID: GaryxFeedbackID,
        scopes: GaryxGatewayScopeRegistry
    ) -> GaryxOperationRemovalFeedbackPlan? {
        guard var entry = snapshot.payloadStore.entry(
                  operationKey.entryID,
                  scope: operationKey.scope
              ),
              entry.operationKeys.contains(operationKey),
              entry.feedbackReferences.contains(feedbackID),
              var operation = snapshot.operations[operationKey],
              var feedback = snapshot.feedback[feedbackID] else {
            return nil
        }
        var lineage = feedback.lineageID.flatMap { snapshot.attachmentLineages[$0] }
        guard GaryxOperationRemovalFeedbackReducer.commit(
            operation: &operation,
            feedback: &feedback,
            lineage: &lineage,
            lifecycle: entry.lifecycle.snapshot,
            scopes: scopes
        ) == .committed else {
            return nil
        }

        var mutations: [GaryxComposerDurabilityMutation] = [
            .upsertOperation(operation),
        ]
        var pendingFileCleanupAssetID: GaryxStagedAssetID?
        if let assetID = snapshot.operations[operationKey]?.stagedAssetID {
            for attachmentID in entry.attachments.values
                .filter({ $0.stagedAssetID == assetID })
                .map(\.id) {
                entry.removeAttachment(attachmentID)
            }
            if snapshot.stagedAssetOwners[assetID] == operationKey {
                if snapshot.pendingFileCleanup[assetID] == nil {
                    mutations.append(.registerFileCleanup(assetID: assetID, owner: operationKey))
                }
                mutations.append(.releaseStagedAsset(assetID))
            }
            if snapshot.pendingFileCleanup[assetID] == operationKey
                || snapshot.stagedAssetOwners[assetID] == operationKey {
                pendingFileCleanupAssetID = assetID
            }
        }
        mutations.append(.removeManifest(operationKey))
        mutations.append(.removeOperation(operationKey))
        mutations.append(.upsertFeedback(feedback))
        if let lineage {
            mutations.append(.upsertAttachmentLineage(lineage))
        }
        entry.removeOperation(operationKey)
        entry.removeFeedbackReference(feedbackID)
        mutations.append(.upsertEntry(entry))

        return GaryxOperationRemovalFeedbackPlan(
            operationKey: operationKey,
            feedbackID: feedbackID,
            pendingFileCleanupAssetID: pendingFileCleanupAssetID,
            transaction: GaryxComposerDurabilityTransaction(
                expectedRevision: snapshot.revision,
                label: "remove failed operation and acknowledge feedback",
                mutations: mutations
            )
        )
    }
}

public enum GaryxComposerDurabilityError: Error, Equatable, Sendable {
    case revisionConflict(expected: UInt64, actual: UInt64)
    case invariantViolation(String)
    case injectedFailure(mutationIndex: Int)
}

/// Typed input for the only durable send linearization operation. It derives
/// the payload mutation from the committed barrier instead of accepting an
/// arbitrary caller-built mutation list, so envelope removal, generation
/// advancement, outbox insertion, and ledger settlement cannot diverge.
public struct GaryxComposerCommitSend: Equatable, Sendable {
    public let expectedRevision: UInt64
    public let ledger: GaryxProvisionalReservationLedger
    public let payloadEntry: GaryxComposerPayloadEntry
    public let barrier: GaryxSendCommitBarrier
    public let delivery: GaryxDeliveryRecord
    public let producerDrained: [
        GaryxSessionDescendantKey: GaryxDurableProducerDrainedRecord
    ]

    public init(
        expectedRevision: UInt64,
        ledger: GaryxProvisionalReservationLedger,
        sealedPayloadEntry: GaryxComposerPayloadEntry,
        barrier: GaryxSendCommitBarrier,
        settlement: GaryxSendBarrierSettlement,
        producerDrained: [
            GaryxSessionDescendantKey: GaryxDurableProducerDrainedRecord
        ] = [:]
    ) throws {
        guard ledger.terminalOutcome == .committed,
              let target = ledger.targetMapping,
              target.entryID == ledger.key.entryID,
              target.generation == ledger.followupGeneration,
              settlement.terminalOutcome == .committed,
              settlement.followupGeneration == ledger.followupGeneration,
              let delivery = settlement.deliveryRecord,
              delivery.scope == ledger.key.scope,
              delivery.entryID == ledger.key.entryID,
              delivery.reservationID == ledger.key.reservationID,
              delivery.phase == .notDispatched,
              delivery.envelope?.generation == ledger.envelopeGeneration,
              barrier.scope == ledger.key.scope,
              barrier.entryID == ledger.key.entryID,
              barrier.reservationID == ledger.key.reservationID,
              barrier.phase == .durableCommitted,
              sealedPayloadEntry.scope == ledger.key.scope,
              sealedPayloadEntry.id == ledger.key.entryID,
              barrier.payloadLifecycle.token == sealedPayloadEntry.lifecycle.token,
              producerDrained.allSatisfy({ key, value in
                  key.token == sealedPayloadEntry.lifecycle.token
                      && value.scope == ledger.key.scope
                      && value.entryID == ledger.key.entryID
                      && value.reservationID == ledger.key.reservationID
              }) else {
            throw GaryxComposerDurabilityError.invariantViolation(
                "commitSend identities or committed reservation outcome do not match"
            )
        }

        var payloadEntry = sealedPayloadEntry
        guard payloadEntry.settleCommittedSend(
            envelopeGeneration: ledger.envelopeGeneration,
            followupGeneration: ledger.followupGeneration,
            followupText: settlement.followupText,
            followupAttachmentIDs: settlement.followupAttachmentIDs,
            deliveryID: delivery.id
        ) else {
            throw GaryxComposerDurabilityError.invariantViolation(
                "commitSend payload cannot advance from the sealed generation"
            )
        }
        self.expectedRevision = expectedRevision
        self.ledger = ledger
        self.payloadEntry = payloadEntry
        self.barrier = barrier
        self.delivery = delivery
        self.producerDrained = producerDrained
    }

    public var transaction: GaryxComposerDurabilityTransaction {
        let drainedMutations = producerDrained
            .sorted { lhs, rhs in
                if lhs.key.epoch != rhs.key.epoch { return lhs.key.epoch < rhs.key.epoch }
                return lhs.key.sessionID.rawValue < rhs.key.sessionID.rawValue
            }
            .map(GaryxComposerDurabilityMutation.upsertProducerDrained)
        return GaryxComposerDurabilityTransaction(
            expectedRevision: expectedRevision,
            label: "commitSend",
            mutations: [
                .upsertLedger(ledger),
            ] + drainedMutations + [
                .upsertEntry(payloadEntry),
                .upsertBarrier(barrier),
                .upsertDelivery(delivery),
            ]
        )
    }
}

/// Single durability seam for composer, payload, operation, reservation, and
/// delivery records. The concrete DB implementation is intentionally deferred
/// to A4d-1.
public protocol GaryxComposerDurabilityStore: Sendable {
    func load() async throws -> GaryxComposerDurabilitySnapshot
    func allocatePayloadGeneration() async throws -> UInt64
    func allocateSendReservationID() async throws -> GaryxSendReservationID
    func commit(
        _ transaction: GaryxComposerDurabilityTransaction
    ) async throws -> GaryxComposerDurabilitySnapshot
    func commitSend(
        _ send: GaryxComposerCommitSend
    ) async throws -> GaryxComposerDurabilitySnapshot
}

/// In-memory transactional fake used by the A3 Core matrix. It applies every
/// mutation to a copy, validates ordering invariants, and publishes the copy
/// only after the whole transaction succeeds.
public actor GaryxFakeComposerDurabilityStore: GaryxComposerDurabilityStore {
    private var state: GaryxComposerDurabilitySnapshot
    private var failAtMutationIndex: Int?
    private var generationAllocator: GaryxDurableHiLoAllocator
    private var reservationAllocator: GaryxDurableHiLoAllocator

    public init(initial: GaryxComposerDurabilitySnapshot = .init()) {
        state = initial
        failAtMutationIndex = nil
        generationAllocator = GaryxDurableHiLoAllocator(
            persistedHighWatermark: initial.generationHighWatermark
        )
        reservationAllocator = GaryxDurableHiLoAllocator(
            persistedHighWatermark: initial.reservationHighWatermark
        )
    }

    public func load() async throws -> GaryxComposerDurabilitySnapshot { state }

    public func allocatePayloadGeneration() async throws -> UInt64 {
        var candidate = generationAllocator
        let value = candidate.allocate()
        if candidate.persistedHighWatermark != generationAllocator.persistedHighWatermark {
            state = try GaryxComposerDurabilityTransactionEngine.applying(
                .init(
                    expectedRevision: state.revision,
                    label: "reserve payload-generation hi-lo block",
                    mutations: [.setGenerationHighWatermark(candidate.persistedHighWatermark)]
                ),
                to: state
            )
        }
        generationAllocator = candidate
        return value
    }

    public func allocateSendReservationID() async throws -> GaryxSendReservationID {
        var candidate = reservationAllocator
        let value = candidate.allocate()
        if candidate.persistedHighWatermark != reservationAllocator.persistedHighWatermark {
            state = try GaryxComposerDurabilityTransactionEngine.applying(
                .init(
                    expectedRevision: state.revision,
                    label: "reserve send-reservation hi-lo block",
                    mutations: [.setReservationHighWatermark(candidate.persistedHighWatermark)]
                ),
                to: state
            )
        }
        reservationAllocator = candidate
        return GaryxSendReservationID(rawValue: value)
    }

    public func injectFailure(atMutationIndex index: Int) {
        precondition(index >= 0)
        failAtMutationIndex = index
    }

    public func commit(
        _ transaction: GaryxComposerDurabilityTransaction
    ) async throws -> GaryxComposerDurabilitySnapshot {
        defer { failAtMutationIndex = nil }
        state = try GaryxComposerDurabilityTransactionEngine.applying(
            transaction,
            to: state,
            failAtMutationIndex: failAtMutationIndex
        )
        return state
    }

    public func commitSend(
        _ send: GaryxComposerCommitSend
    ) async throws -> GaryxComposerDurabilitySnapshot {
        try await commit(send.transaction)
    }
}

/// Shared value transaction engine used by both the A3 fake and the concrete
/// SQLite store. Persistence implementations own publication and fsync; this
/// reducer owns mutation ordering, CAS, and cross-record invariants.
enum GaryxComposerDurabilityTransactionEngine {
    static func applying(
        _ transaction: GaryxComposerDurabilityTransaction,
        to state: GaryxComposerDurabilitySnapshot,
        failAtMutationIndex: Int? = nil,
        afterApplyingMutation: ((Int) throws -> Void)? = nil
    ) throws -> GaryxComposerDurabilitySnapshot {
        guard transaction.expectedRevision == state.revision else {
            throw GaryxComposerDurabilityError.revisionConflict(
                expected: transaction.expectedRevision,
                actual: state.revision
            )
        }
        var candidate = state
        for (index, mutation) in transaction.mutations.enumerated() {
            if failAtMutationIndex == index {
                throw GaryxComposerDurabilityError.injectedFailure(mutationIndex: index)
            }
            try apply(mutation, to: &candidate)
            try afterApplyingMutation?(index)
        }
        compactCorrelationTombstonesToBudget(in: &candidate)
        compactRetiredReservationLedgers(in: &candidate)
        try validate(candidate)
        candidate.revision &+= 1
        return candidate
    }

    private static func apply(
        _ mutation: GaryxComposerDurabilityMutation,
        to state: inout GaryxComposerDurabilitySnapshot
    ) throws {
        switch mutation {
        case .upsertEntry(let entry):
            if let existing = state.payloadStore.entry(entry.id, scope: entry.scope) {
                guard entry.lifecycle.token == existing.lifecycle.token,
                      entry.lifecycle.revision >= existing.lifecycle.revision,
                      entry.currentGeneration >= existing.currentGeneration,
                      lifecycleRank(entry.lifecycle.phase) >= lifecycleRank(existing.lifecycle.phase) else {
                    throw invariant("entry identity, lifecycle, or generation regressed")
                }
                state.payloadStore.update(entry)
            } else {
                guard state.discardConvergence[entry.id] == nil else {
                    throw invariant("discarded payload identity cannot be reinserted")
                }
                guard state.payloadStore.insert(entry) else {
                    throw invariant("entry insert failed")
                }
            }
        case .removeEntry(let scope, let entryID):
            _ = state.payloadStore.remove(entryID, scope: scope)
        case .replaceAliases(let aliases):
            state.aliases = aliases
        case .upsertOperation(let operation):
            try requireLedgerIfNeeded(
                scope: operation.context.key.scope,
                entryID: operation.context.key.entryID,
                reservationID: operation.context.key.reservationID,
                state: state,
                descendant: "operation capability"
            )
            state.operations[operation.context.key] = operation
        case .removeOperation(let key):
            state.operations.removeValue(forKey: key)
        case .upsertManifest(let manifest):
            try requireLedgerIfNeeded(
                scope: manifest.key.scope,
                entryID: manifest.key.entryID,
                reservationID: manifest.key.reservationID,
                state: state,
                descendant: "operation manifest"
            )
            state.manifests[manifest.key] = manifest
        case .removeManifest(let key):
            state.manifests.removeValue(forKey: key)
        case .upsertReplacement(let replacement):
            try requireLedgerIfNeeded(
                scope: replacement.scope,
                entryID: replacement.entryID,
                reservationID: replacement.reservationID,
                state: state,
                descendant: "replacement record"
            )
            state.replacements[replacement.id] = replacement
        case .removeReplacement(let id):
            state.replacements.removeValue(forKey: id)
        case .upsertConflict(let conflict):
            state.conflicts[conflict.id] = conflict
        case .removeConflict(let id):
            state.conflicts.removeValue(forKey: id)
        case .upsertFeedback(let feedback):
            state.feedback[feedback.id] = feedback
        case .removeFeedback(let id):
            state.feedback.removeValue(forKey: id)
        case .upsertAttachmentLineage(let lineage):
            state.attachmentLineages[lineage.id] = lineage
        case .removeAttachmentLineage(let id):
            state.attachmentLineages.removeValue(forKey: id)
        case .upsertBarrier(let barrier):
            try requireLedgerIfNeeded(
                scope: barrier.scope,
                entryID: barrier.entryID,
                reservationID: barrier.reservationID,
                state: state,
                descendant: "send barrier"
            )
            state.barriers[barrier.entryID] = barrier
        case .removeBarrier(let entryID):
            guard let barrier = state.barriers[entryID],
                  barrier.phase == .idle,
                  barrier.reservationID == nil,
                  barrier.envelopeGeneration == nil,
                  barrier.followupGeneration == nil,
                  barrier.envelopeText == nil,
                  barrier.envelopeAttachmentIDs.isEmpty,
                  barrier.envelopeClientIntentID == nil,
                  barrier.provisionalFollowupText.isEmpty,
                  barrier.provisionalFollowupAttachmentIDs.isEmpty else {
                throw invariant("send barrier GC requires idle phase")
            }
            state.barriers.removeValue(forKey: entryID)
        case .upsertLedger(let ledger):
            state.ledgers[ledger.key] = ledger
        case .removeLedger(let key):
            guard state.ledgers[key]?.terminalOutcome != nil,
                  !ledgerHasDurableDescendant(key, state: state) else {
                throw invariant("reservation ledger GC requires terminal descendant-free state")
            }
            state.ledgers.removeValue(forKey: key)
        case .synthesizeReservationRevocation(let key):
            guard var ledger = state.ledgers[key],
                  ledger.synthesizeTerminalOutcome(.revoked) else {
                throw invariant("synthetic revocation requires an unsettled reservation ledger")
            }
            state.ledgers[key] = ledger
        case .persistReservationTargetMapping(let key, let generation):
            guard var ledger = state.ledgers[key],
                  ledger.persistTargetMapping(generation) else {
                throw invariant("reservation target mapping does not match its terminal outcome")
            }
            state.ledgers[key] = ledger
        case .upsertProducerDrained(let key, let drained):
            guard key.token.entryID == drained.entryID,
                  key.sessionID == drained.record.sessionID,
                  key.epoch == drained.record.epoch else {
                throw invariant("producerDrained identity is inconsistent")
            }
            try requireLedgerIfNeeded(
                scope: drained.scope,
                entryID: drained.entryID,
                reservationID: drained.reservationID,
                state: state,
                descendant: "producerDrained"
            )
            state.producerDrained[key] = drained
        case .removeProducerDrained(let key):
            state.producerDrained.removeValue(forKey: key)
        case .upsertRecoveredInputClose(let close):
            guard close.key.token.entryID == close.entryID,
                  close.closePublicationCount == 1 else {
                throw invariant("recovered input close identity is inconsistent")
            }
            state.recoveredInputClosures[close.key] = close
        case .removeRecoveredInputClose(let key):
            state.recoveredInputClosures.removeValue(forKey: key)
        case .upsertDelivery(let delivery):
            let ledgerKey = GaryxReservationLedgerKey(
                scope: delivery.scope,
                entryID: delivery.entryID,
                reservationID: delivery.reservationID
            )
            guard state.ledgers[ledgerKey]?.terminalOutcome == .committed else {
                throw invariant("delivery requires committed reservation ledger")
            }
            if let existing = state.deliveries[delivery.id],
               !delivery.durablyAdvances(from: existing) {
                throw invariant("delivery phase or evidence regressed")
            }
            state.deliveries[delivery.id] = delivery
        case .removeDelivery(let id):
            state.deliveries.removeValue(forKey: id)
        case .upsertDiscardConvergence(let convergence):
            state.discardConvergence[convergence.lifecycle.token.entryID] = convergence
        case .removeDiscardConvergence(let entryID):
            guard state.discardConvergence[entryID]?.lifecycle.phase == .discarded else {
                throw invariant("discard convergence GC requires discarded lifecycle")
            }
            state.discardConvergence.removeValue(forKey: entryID)
        case .upsertCreateDelivery(let create):
            state.createDeliveries[create.key] = create
        case .removeCreateDelivery(let key):
            guard state.createDeliveries[key]?.isTerminalCorrelation == true else {
                throw invariant("create delivery GC requires terminal correlation")
            }
            state.createDeliveries.removeValue(forKey: key)
        case .replaceScopeRegistry(let registry):
            state.scopeRegistry = registry
        case .reserveStagedAsset(let assetID, let owner, let bytes):
            guard bytes >= 0 else { throw invariant("negative asset reservation") }
            if let existing = state.stagedAssetOwners[assetID], existing != owner {
                throw invariant("staged asset has multiple owners")
            }
            let previousBytes = state.stagedAssetReservedBytes[assetID] ?? 0
            state.reservedBytes += bytes - previousBytes
            state.stagedAssetOwners[assetID] = owner
            state.stagedAssetReservedBytes[assetID] = bytes
        case .releaseStagedAsset(let assetID):
            state.stagedAssetOwners.removeValue(forKey: assetID)
            state.reservedBytes -= state.stagedAssetReservedBytes.removeValue(forKey: assetID) ?? 0
        case .registerFileCleanup(let assetID, let owner):
            guard state.pendingFileCleanup[assetID] == nil
                    || state.pendingFileCleanup[assetID] == owner else {
                throw invariant("staged file cleanup has multiple owners")
            }
            state.pendingFileCleanup[assetID] = owner
        case .completeFileCleanup(let assetID):
            state.pendingFileCleanup.removeValue(forKey: assetID)
        case .setGenerationHighWatermark(let watermark):
            guard watermark >= state.generationHighWatermark else {
                throw invariant("generation watermark regressed")
            }
            if watermark > state.generationHighWatermark {
                state.generationClaimFloor = max(
                    state.generationClaimFloor,
                    state.generationHighWatermark
                )
                state.claimedGenerations = state.claimedGenerations.filter {
                    $0 > state.generationClaimFloor
                }
            }
            state.generationHighWatermark = watermark
        case .claimGeneration(let generation):
            guard generation > 0,
                  generation > state.generationClaimFloor,
                  generation <= state.generationHighWatermark,
                  state.claimedGenerations.insert(generation).inserted else {
                throw invariant("generation was not durably allocated or was already claimed")
            }
        case .setReservationHighWatermark(let watermark):
            guard watermark >= state.reservationHighWatermark else {
                throw invariant("reservation watermark regressed")
            }
            state.reservationHighWatermark = watermark
        }
    }

    private static func requireLedgerIfNeeded(
        scope: GaryxGatewayScope,
        entryID: GaryxComposerPayloadEntryID,
        reservationID: GaryxSendReservationID?,
        state: GaryxComposerDurabilitySnapshot,
        descendant: String
    ) throws {
        guard let reservationID else { return }
        let key = GaryxReservationLedgerKey(
            scope: scope,
            entryID: entryID,
            reservationID: reservationID
        )
        guard state.ledgers[key] != nil else {
            throw invariant("\(descendant) requires reservation ledger first")
        }
    }

    private static func validate(_ state: GaryxComposerDurabilitySnapshot) throws {
        let tombstoneUsage = state.persistentTombstoneUsage
        guard state.tombstoneBudget.admits(
            count: tombstoneUsage.count,
            bytes: tombstoneUsage.bytes
        ) else {
            throw invariant("persistent tombstone budget exceeded")
        }
        guard state.generationClaimFloor <= state.generationHighWatermark,
              state.claimedGenerations.count <= Int(GaryxDurableHiLoAllocator.maximumBlockSize),
              state.claimedGenerations.allSatisfy({
                  $0 > state.generationClaimFloor && $0 <= state.generationHighWatermark
              }) else {
            throw invariant("claimed generation is outside the durable hi-lo watermark")
        }
        for (key, operation) in state.operations {
            guard operation.context.key == key,
                  let entry = state.payloadStore.entry(key.entryID, scope: key.scope),
                  entry.operationKeys.contains(key),
                  entry.lifecycle.token == operation.context.payloadLifecycle.token else {
                throw invariant("operation capability is absent from authoritative Entry membership")
            }
            if operation.state == .superseded,
               operation.stagedAssetID != nil || operation.reservedBytes != 0 {
                throw invariant("superseded operation must retain lineage only")
            }
        }
        for (key, manifest) in state.manifests {
            guard manifest.key == key,
                  let entry = state.payloadStore.entry(key.entryID, scope: key.scope),
                  entry.operationKeys.contains(key) else {
                throw invariant("operation manifest is absent from authoritative Entry membership")
            }
        }
        for (scope, entries) in state.payloadStore.entriesByScope {
            for entry in entries.values {
                for key in entry.operationKeys {
                    guard key.scope == scope,
                          key.entryID == entry.id,
                          state.operations[key] != nil || state.manifests[key] != nil else {
                        throw invariant("Entry operation membership has no durable descendant")
                    }
                }
            }
        }
        for (entryID, convergence) in state.discardConvergence {
            guard convergence.lifecycle.token.entryID == entryID else {
                throw invariant("discard convergence is stored under the wrong Entry identity")
            }
            if convergence.lifecycle.phase == .discarded,
               state.payloadStore.entry(entryID, scope: convergence.barrier.scope) != nil {
                throw invariant("discarded payload identity cannot coexist with an Entry")
            }
        }
        for ledger in state.ledgers.values {
            guard (ledger.terminalOutcome == nil) == (ledger.targetMapping == nil) else {
                throw invariant("reservation outcome and target mapping must publish together")
            }
        }
        for barrier in state.barriers.values where barrier.phase == .durableCommitted {
            guard let reservationID = barrier.reservationID,
                  state.deliveries.values.contains(where: {
                      $0.scope == barrier.scope
                          && $0.entryID == barrier.entryID
                          && $0.reservationID == reservationID
                  }) else {
                throw invariant(
                    "durable-committed send barrier requires matching reservation delivery"
                )
            }
        }
        let unsettledLedgers = state.ledgers.values.filter { $0.terminalOutcome == nil }
        let unsettledLedgerBytes = unsettledLedgers.reduce(0) { $0 + $1.estimatedBytes }
        let unsettledLedgersByScope = Dictionary(grouping: unsettledLedgers) { $0.key.scope }
        guard unsettledLedgers.count <= GaryxProvisionalReservationLedger.unsettledGlobalLimit,
              unsettledLedgerBytes <= GaryxProvisionalReservationLedger.unsettledByteLimit,
              unsettledLedgersByScope.values.allSatisfy({
                  $0.count <= GaryxProvisionalReservationLedger.unsettledPerScopeLimit
              }) else {
            throw invariant("unsettled reservation ledger budget exceeded")
        }
        let nonTerminalCreates = state.createDeliveries.values.filter {
            !$0.isTerminalCorrelation
        }
        let nonTerminalCreateBytes = nonTerminalCreates.reduce(0) { $0 + $1.estimatedBytes }
        let nonTerminalCreatesByScope = Dictionary(grouping: nonTerminalCreates, by: \.scope)
        guard nonTerminalCreates.count <= GaryxCreateDeliveryState.nonTerminalGlobalLimit,
              nonTerminalCreateBytes <= GaryxCreateDeliveryState.nonTerminalByteLimit,
              nonTerminalCreatesByScope.values.allSatisfy({
                  $0.count <= GaryxCreateDeliveryState.nonTerminalPerScopeLimit
              }),
              state.createDeliveries.allSatisfy({ $0.key == $0.value.key }) else {
            throw invariant("non-terminal create delivery budget or identity exceeded")
        }
        if let active = state.scopeRegistry.activeScope {
            guard state.scopeRegistry.lifecycle(of: active) == .active,
                  state.scopeRegistry.lifecycles.values.filter({ $0 == .active }).count == 1 else {
                throw invariant("scope registry active identity is inconsistent")
            }
        } else if state.scopeRegistry.lifecycles.values.contains(.active) {
            throw invariant("scope registry has an ownerless active partition")
        }
        guard state.scopeRegistry.lifecycles.keys.allSatisfy({ scope in
            scope.epoch > (state.scopeRegistry.revokedThroughEpoch[scope.identity] ?? 0)
        }) else {
            throw invariant("scope registry retained an epoch at or below its revoke watermark")
        }
        for close in state.recoveredInputClosures.values {
            let ledgerKey = GaryxReservationLedgerKey(
                scope: close.scope,
                entryID: close.entryID,
                reservationID: close.reservationID
            )
            guard state.ledgers[ledgerKey]?.terminalOutcome == .revoked,
                  state.ledgers[ledgerKey]?.targetMapping?.generation == close.targetGeneration,
                  state.producerDrained[close.key] == nil else {
                throw invariant("recovered close requires a consumed drained record and revoked mapping")
            }
        }
        guard state.reservedBytes >= 0,
              Set(state.stagedAssetOwners.keys) == Set(state.stagedAssetReservedBytes.keys),
              state.stagedAssetReservedBytes.values.reduce(0, +) == state.reservedBytes else {
            throw invariant("staged asset quota metadata is inconsistent")
        }
        for (assetID, owner) in state.stagedAssetOwners {
            guard let operation = state.operations[owner],
                  operation.stagedAssetID == assetID,
                  operation.reservedBytes == state.stagedAssetReservedBytes[assetID] else {
                throw invariant("staged asset owner does not match operation")
            }
        }
        for (assetID, owner) in state.pendingFileCleanup {
            if let operation = state.operations[owner] {
                guard operation.state == .failedTerminal,
                      operation.stagedAssetID == assetID else {
                    throw invariant("pending file cleanup does not match failed-terminal operation")
                }
            } else {
                // Removing/resetting the operation must not erase the only
                // durable proof that its physical staged file still needs
                // deletion. This condemned tombstone is retired solely by a
                // later `.completeFileCleanup` acknowledgement.
                guard state.stagedAssetOwners[assetID] == nil,
                      !state.operations.values.contains(where: { $0.stagedAssetID == assetID }) else {
                    throw invariant("condemned file cleanup still has a live operation owner")
                }
            }
        }
        for lineage in state.attachmentLineages.values {
            guard let feedback = state.feedback[lineage.feedbackID],
                  feedback.lineageID == lineage.id,
                  feedback.scope == lineage.scope,
                  feedback.entryID == lineage.entryID else {
                throw invariant("attachment lineage does not match feedback")
            }
            if lineage.phase == .released, !feedback.isTerminal {
                throw invariant("released attachment lineage requires terminal feedback")
            }
        }
    }

    /// Terminal send/create rows are bounded correlation evidence, not
    /// immortal history. Create rows are retired first by stable identity;
    /// delivery rows retain their mandated `(ReservationID, DeliveryRecordID)`
    /// order. Non-terminal rows and discard-finalization tombstones are never
    /// selected, so an unprunable overage remains fail-closed.
    private static func compactCorrelationTombstonesToBudget(
        in state: inout GaryxComposerDurabilitySnapshot
    ) {
        var usage = state.persistentTombstoneUsage
        guard !state.tombstoneBudget.admits(count: usage.count, bytes: usage.bytes) else {
            return
        }
        let createCandidates = state.createDeliveries.values.compactMap {
            record -> (GaryxCreateDeliveryState, Int)? in
            guard let bytes = record.persistentTombstoneEstimatedBytes else { return nil }
            return (record, bytes)
        }.sorted { lhs, rhs in
            if lhs.0.scope.identity != rhs.0.scope.identity {
                return lhs.0.scope.identity < rhs.0.scope.identity
            }
            if lhs.0.scope.epoch != rhs.0.scope.epoch {
                return lhs.0.scope.epoch < rhs.0.scope.epoch
            }
            return lhs.0.createIntentID < rhs.0.createIntentID
        }
        for (record, bytes) in createCandidates {
            guard !state.tombstoneBudget.admits(count: usage.count, bytes: usage.bytes) else {
                break
            }
            state.createDeliveries.removeValue(forKey: record.key)
            usage = GaryxPersistentTombstoneUsage(
                correlationCount: usage.correlationCount,
                correlationBytes: usage.correlationBytes,
                createCorrelationCount: usage.createCorrelationCount - 1,
                createCorrelationBytes: usage.createCorrelationBytes - bytes,
                discardFinalizationCount: usage.discardFinalizationCount,
                discardFinalizationBytes: usage.discardFinalizationBytes
            )
        }
        let candidates = state.deliveries.values.compactMap { record -> (GaryxDeliveryRecord, Int)? in
            guard let bytes = record.persistentTombstoneEstimatedBytes else { return nil }
            return (record, bytes)
        }.sorted { lhs, rhs in
            if lhs.0.reservationID != rhs.0.reservationID {
                return lhs.0.reservationID.rawValue < rhs.0.reservationID.rawValue
            }
            return lhs.0.id.rawValue < rhs.0.id.rawValue
        }
        for (record, bytes) in candidates {
            guard !state.tombstoneBudget.admits(count: usage.count, bytes: usage.bytes) else {
                break
            }
            state.deliveries.removeValue(forKey: record.id)
            usage = GaryxPersistentTombstoneUsage(
                correlationCount: usage.correlationCount - 1,
                correlationBytes: usage.correlationBytes - bytes,
                createCorrelationCount: usage.createCorrelationCount,
                createCorrelationBytes: usage.createCorrelationBytes,
                discardFinalizationCount: usage.discardFinalizationCount,
                discardFinalizationBytes: usage.discardFinalizationBytes
            )
        }
    }

    /// Once a reservation has a terminal mapping and no durable descendant
    /// can consume it, absence is the protocol's stable unknown-reservation
    /// fence. Retire it in the same transaction that removes its last child.
    private static func compactRetiredReservationLedgers(
        in state: inout GaryxComposerDurabilitySnapshot
    ) {
        let retired = state.ledgers.keys.filter { key in
            state.ledgers[key]?.terminalOutcome != nil
                && !ledgerHasDurableDescendant(key, state: state)
        }
        for key in retired {
            state.ledgers.removeValue(forKey: key)
        }
    }

    private static func ledgerHasDurableDescendant(
        _ key: GaryxReservationLedgerKey,
        state: GaryxComposerDurabilitySnapshot
    ) -> Bool {
        if state.barriers.values.contains(where: {
            $0.scope == key.scope && $0.entryID == key.entryID
                && $0.reservationID == key.reservationID
        }) {
            return true
        }
        if state.operations.keys.contains(where: {
            $0.scope == key.scope && $0.entryID == key.entryID
                && $0.reservationID == key.reservationID
        }) || state.manifests.keys.contains(where: {
            $0.scope == key.scope && $0.entryID == key.entryID
                && $0.reservationID == key.reservationID
        }) {
            return true
        }
        if state.replacements.values.contains(where: {
            $0.scope == key.scope && $0.entryID == key.entryID
                && $0.reservationID == key.reservationID
        }) || state.producerDrained.values.contains(where: {
            $0.scope == key.scope && $0.entryID == key.entryID
                && $0.reservationID == key.reservationID
        }) || state.recoveredInputClosures.values.contains(where: {
            $0.scope == key.scope && $0.entryID == key.entryID
                && $0.reservationID == key.reservationID
        }) || state.deliveries.values.contains(where: {
            $0.scope == key.scope && $0.entryID == key.entryID
                && $0.reservationID == key.reservationID
        }) {
            return true
        }
        return state.discardConvergence.values.contains { convergence in
            guard convergence.barrier.scope == key.scope,
                  convergence.lifecycle.token.entryID == key.entryID else {
                return false
            }
            return convergence.barrier.reservationID == key.reservationID
                || convergence.operations.keys.contains(where: {
                    $0.reservationID == key.reservationID
                })
                || convergence.replacements.values.contains(where: {
                    $0.reservationID == key.reservationID
                })
                || convergence.deliveries.values.contains(where: {
                    $0.reservationID == key.reservationID
                })
        }
    }

    private static func invariant(_ message: String) -> GaryxComposerDurabilityError {
        .invariantViolation(message)
    }

    private static func lifecycleRank(_ phase: GaryxPayloadLifecyclePhase) -> Int {
        switch phase {
        case .active: 0
        case .discarding: 1
        case .discarded: 2
        }
    }
}
