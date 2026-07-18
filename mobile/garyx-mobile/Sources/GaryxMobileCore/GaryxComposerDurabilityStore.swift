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
    public fileprivate(set) var deliveries: [GaryxDeliveryRecordID: GaryxDeliveryRecord]
    public fileprivate(set) var discardConvergence: [GaryxComposerPayloadEntryID: GaryxPayloadDiscardConvergence]
    public fileprivate(set) var createDeliveries: [GaryxCreateDeliveryKey: GaryxCreateDeliveryState]
    public fileprivate(set) var stagedAssetOwners: [GaryxStagedAssetID: GaryxOperationCapabilityKey]
    public fileprivate(set) var stagedAssetReservedBytes: [GaryxStagedAssetID: Int]
    public fileprivate(set) var reservedBytes: Int
    public fileprivate(set) var generationHighWatermark: UInt64
    public fileprivate(set) var reservationHighWatermark: UInt64

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
        deliveries: [GaryxDeliveryRecordID: GaryxDeliveryRecord] = [:],
        discardConvergence: [GaryxComposerPayloadEntryID: GaryxPayloadDiscardConvergence] = [:],
        createDeliveries: [GaryxCreateDeliveryKey: GaryxCreateDeliveryState] = [:],
        stagedAssetOwners: [GaryxStagedAssetID: GaryxOperationCapabilityKey] = [:],
        stagedAssetReservedBytes: [GaryxStagedAssetID: Int] = [:],
        reservedBytes: Int = 0,
        generationHighWatermark: UInt64 = 0,
        reservationHighWatermark: UInt64 = 0
    ) {
        precondition(reservedBytes >= 0)
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
        self.deliveries = deliveries
        self.discardConvergence = discardConvergence
        self.createDeliveries = createDeliveries
        self.stagedAssetOwners = stagedAssetOwners
        self.stagedAssetReservedBytes = stagedAssetReservedBytes
        self.reservedBytes = reservedBytes
        self.generationHighWatermark = generationHighWatermark
        self.reservationHighWatermark = reservationHighWatermark
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
    case upsertLedger(GaryxProvisionalReservationLedger)
    case upsertProducerDrained(GaryxSessionDescendantKey, GaryxDurableProducerDrainedRecord)
    case upsertDelivery(GaryxDeliveryRecord)
    case removeDelivery(GaryxDeliveryRecordID)
    case upsertDiscardConvergence(GaryxPayloadDiscardConvergence)
    case removeDiscardConvergence(GaryxComposerPayloadEntryID)
    case upsertCreateDelivery(GaryxCreateDeliveryState)
    case reserveStagedAsset(
        assetID: GaryxStagedAssetID,
        owner: GaryxOperationCapabilityKey,
        bytes: Int
    )
    case releaseStagedAsset(GaryxStagedAssetID)
    case setGenerationHighWatermark(UInt64)
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

public enum GaryxComposerDurabilityError: Error, Equatable, Sendable {
    case revisionConflict(expected: UInt64, actual: UInt64)
    case invariantViolation(String)
    case injectedFailure(mutationIndex: Int)
}

/// Single durability seam for composer, payload, operation, reservation, and
/// delivery records. The concrete DB implementation is intentionally deferred
/// to A4d-1.
public protocol GaryxComposerDurabilityStore: Sendable {
    func load() async throws -> GaryxComposerDurabilitySnapshot
    func commit(
        _ transaction: GaryxComposerDurabilityTransaction
    ) async throws -> GaryxComposerDurabilitySnapshot
}

/// In-memory transactional fake used by the A3 Core matrix. It applies every
/// mutation to a copy, validates ordering invariants, and publishes the copy
/// only after the whole transaction succeeds.
public actor GaryxFakeComposerDurabilityStore: GaryxComposerDurabilityStore {
    private var state: GaryxComposerDurabilitySnapshot
    private var failAtMutationIndex: Int?

    public init(initial: GaryxComposerDurabilitySnapshot = .init()) {
        state = initial
        failAtMutationIndex = nil
    }

    public func load() async throws -> GaryxComposerDurabilitySnapshot { state }

    public func injectFailure(atMutationIndex index: Int) {
        precondition(index >= 0)
        failAtMutationIndex = index
    }

    public func commit(
        _ transaction: GaryxComposerDurabilityTransaction
    ) async throws -> GaryxComposerDurabilitySnapshot {
        guard transaction.expectedRevision == state.revision else {
            throw GaryxComposerDurabilityError.revisionConflict(
                expected: transaction.expectedRevision,
                actual: state.revision
            )
        }
        var candidate = state
        for (index, mutation) in transaction.mutations.enumerated() {
            if failAtMutationIndex == index {
                failAtMutationIndex = nil
                throw GaryxComposerDurabilityError.injectedFailure(mutationIndex: index)
            }
            try apply(mutation, to: &candidate)
        }
        try validate(candidate)
        candidate.revision &+= 1
        state = candidate
        failAtMutationIndex = nil
        return state
    }

    private func apply(
        _ mutation: GaryxComposerDurabilityMutation,
        to state: inout GaryxComposerDurabilitySnapshot
    ) throws {
        switch mutation {
        case .upsertEntry(let entry):
            if state.payloadStore.entry(entry.id, scope: entry.scope) == nil {
                guard state.payloadStore.insert(entry) else {
                    throw invariant("entry insert failed")
                }
            } else {
                state.payloadStore.update(entry)
            }
        case .removeEntry(let scope, let entryID):
            _ = state.payloadStore.remove(entryID, scope: scope)
        case .replaceAliases(let aliases):
            state.aliases = aliases
        case .upsertOperation(let operation):
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
            state.barriers[barrier.entryID] = barrier
        case .upsertLedger(let ledger):
            state.ledgers[ledger.key] = ledger
        case .upsertProducerDrained(let key, let drained):
            try requireLedgerIfNeeded(
                scope: drained.scope,
                entryID: drained.entryID,
                reservationID: drained.reservationID,
                state: state,
                descendant: "producerDrained"
            )
            state.producerDrained[key] = drained
        case .upsertDelivery(let delivery):
            let ledgerKey = GaryxReservationLedgerKey(
                scope: delivery.scope,
                entryID: delivery.entryID,
                reservationID: delivery.reservationID
            )
            guard state.ledgers[ledgerKey]?.terminalOutcome == .committed else {
                throw invariant("delivery requires committed reservation ledger")
            }
            state.deliveries[delivery.id] = delivery
        case .removeDelivery(let id):
            state.deliveries.removeValue(forKey: id)
        case .upsertDiscardConvergence(let convergence):
            state.discardConvergence[convergence.lifecycle.token.entryID] = convergence
        case .removeDiscardConvergence(let entryID):
            state.discardConvergence.removeValue(forKey: entryID)
        case .upsertCreateDelivery(let create):
            state.createDeliveries[create.key] = create
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
        case .setGenerationHighWatermark(let watermark):
            guard watermark >= state.generationHighWatermark else {
                throw invariant("generation watermark regressed")
            }
            state.generationHighWatermark = watermark
        case .setReservationHighWatermark(let watermark):
            guard watermark >= state.reservationHighWatermark else {
                throw invariant("reservation watermark regressed")
            }
            state.reservationHighWatermark = watermark
        }
    }

    private func requireLedgerIfNeeded(
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

    private func validate(_ state: GaryxComposerDurabilitySnapshot) throws {
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

    private func invariant(_ message: String) -> GaryxComposerDurabilityError {
        .invariantViolation(message)
    }
}
