import Foundation

// MARK: - Atomic ambiguous-delivery exits

public struct GaryxDeliveryDraftRecoveryPlan: Equatable, Sendable {
    public let envelope: GaryxDeliveryEnvelope
    public let placement: GaryxRecoveredDraftPlacement
    public let unrestoredAttachmentIDs: [GaryxAttachmentID]
    public let transaction: GaryxComposerDurabilityTransaction
}

public enum GaryxRecoveredDraftPlacement: Equatable, Sendable {
    case adoptedIntoHost
    case deferred(
        entryID: GaryxComposerPayloadEntryID,
        conflictSetID: GaryxPayloadConflictSetID
    )
}

public enum GaryxDeliveryDraftRecoveryPlanner {
    /// Restores an envelope without interrupting a newer draft. An empty host
    /// with no older deferred recovery adopts the payload in this transaction.
    /// A meaningful host, or one already waiting for an earlier recovery,
    /// remains byte for byte unchanged while the new recovered payload is
    /// durably deferred for automatic FIFO adoption.
    public static func plan(
        snapshot: GaryxComposerDurabilitySnapshot,
        deliveryID: GaryxDeliveryRecordID,
        recoveredEntryID: GaryxComposerPayloadEntryID,
        recoveredLifecycleNonce: String,
        recoveredGeneration: UInt64,
        conflictSetID: GaryxPayloadConflictSetID,
        allowingUndispatched: Bool = false,
        incompleteAttachmentFeedbackID: GaryxFeedbackID? = nil
    ) -> GaryxDeliveryDraftRecoveryPlan? {
        guard recoveredGeneration > snapshot.generationClaimFloor,
              recoveredGeneration <= snapshot.generationHighWatermark,
              !snapshot.claimedGenerations.contains(recoveredGeneration),
              var record = snapshot.deliveries[deliveryID],
              let originalEnvelope = record.envelope,
              var hostEntry = snapshot.payloadStore.entry(
                record.entryID,
                scope: record.scope
              ),
              hostEntry.lifecycle.phase == .active,
              snapshot.payloadStore.entry(recoveredEntryID, scope: record.scope) == nil,
              !recoveredLifecycleNonce.isEmpty else {
            return nil
        }
        let envelopeAttachmentIDs = Set(originalEnvelope.attachmentIDs)
        let snapshotAttachmentIDs = Set(originalEnvelope.attachments.map(\.id))
        let unrestoredAttachmentIDs = originalEnvelope.attachmentIDs.filter {
            !snapshotAttachmentIDs.contains($0)
        }
        guard snapshotAttachmentIDs.isSubset(of: envelopeAttachmentIDs),
              unrestoredAttachmentIDs.isEmpty || incompleteAttachmentFeedbackID != nil,
              incompleteAttachmentFeedbackID.map({ snapshot.feedback[$0] == nil }) ?? true else {
            return nil
        }

        guard case .restored(let envelope) = GaryxDeliveryDraftRecoveryReducer.restore(
            record: &record,
            allowingUndispatched: allowingUndispatched
        ) else {
            return nil
        }

        let restoredAttachments = envelope.attachments.map { attachment in
            GaryxComposerAttachment(
                id: attachment.id,
                stagedAssetID: attachment.stagedAssetID,
                generation: recoveredGeneration,
                byteCount: attachment.byteCount,
                kind: attachment.kind,
                name: attachment.name,
                mediaType: attachment.mediaType,
                uploadedPath: attachment.uploadedPath,
                previewDataURL: attachment.previewDataURL
            )
        }
        hostEntry.removeDeliveryReference(deliveryID)
        var feedback: GaryxOperationFeedback?
        if !unrestoredAttachmentIDs.isEmpty,
           let feedbackID = incompleteAttachmentFeedbackID {
            feedback = GaryxOperationFeedback(
                id: feedbackID,
                scope: record.scope,
                entryID: hostEntry.id,
                operationID: nil,
                kind: .deliveryAttachmentRecoveryIncomplete
            )
            hostEntry.addFeedbackReference(feedbackID)
        }

        let placement: GaryxRecoveredDraftPlacement
        var mutations: [GaryxComposerDurabilityMutation] = [
            .claimGeneration(recoveredGeneration),
        ]
        let hasEarlierDeferredRecovery = GaryxDeferredDraftAdoptionPlanner.candidate(
            snapshot: snapshot,
            hostEntryID: hostEntry.id
        ) != nil
        if hostEntry.hasMeaningfulCurrentPayload || hasEarlierDeferredRecovery {
            var conflict = snapshot.conflicts[conflictSetID]
                ?? GaryxPayloadConflictSet(id: conflictSetID, scope: record.scope)
            guard conflict.scope == record.scope,
                  conflict.admitCandidate(
                    .init(entryID: hostEntry.id, label: "Current draft"),
                    membershipDurabilityAvailable: true
                  ),
                  conflict.admitCandidate(
                    .init(entryID: recoveredEntryID, label: "Recovered send"),
                    membershipDurabilityAvailable: true
                  ) else {
                return nil
            }
            var recoveredEntry = GaryxComposerPayloadEntry(
                id: recoveredEntryID,
                scope: record.scope,
                destination: .draft("delivery-recovery-\(recoveredEntryID.rawValue)"),
                lifecycleToken: GaryxPayloadLifecycleToken(
                    entryID: recoveredEntryID,
                    nonce: recoveredLifecycleNonce
                ),
                currentGeneration: recoveredGeneration,
                text: envelope.text
            )
            for attachment in restoredAttachments {
                recoveredEntry.addAttachment(attachment)
            }
            placement = .deferred(entryID: recoveredEntryID, conflictSetID: conflictSetID)
            mutations.append(contentsOf: [
                .upsertEntry(hostEntry),
                .upsertEntry(recoveredEntry),
                .upsertConflict(conflict),
                .upsertDelivery(record),
            ])
        } else {
            guard hostEntry.replaceCurrentPayload(
                text: envelope.text,
                attachments: restoredAttachments,
                generation: recoveredGeneration
            ) else {
                return nil
            }
            placement = .adoptedIntoHost
            mutations.append(contentsOf: [
                .upsertEntry(hostEntry),
                .upsertDelivery(record),
            ])
        }
        if let feedback {
            mutations.append(.upsertFeedback(feedback))
        }
        return GaryxDeliveryDraftRecoveryPlan(
            envelope: envelope,
            placement: placement,
            unrestoredAttachmentIDs: unrestoredAttachmentIDs,
            transaction: GaryxComposerDurabilityTransaction(
                expectedRevision: snapshot.revision,
                label: "restore delivery with automatic payload placement",
                mutations: mutations
            )
        )
    }
}

public enum GaryxUndispatchedDeliveryRecoveryPlanner {
    /// A bare `notDispatched` record has not crossed the transport gate. It is
    /// therefore safe to restore without duplicate risk. Deliveries owned by
    /// an unfinished multi-stage create retain that create's explicit exit.
    public static func plan(
        snapshot: GaryxComposerDurabilitySnapshot,
        deliveryID: GaryxDeliveryRecordID,
        recoveredEntryID: GaryxComposerPayloadEntryID,
        recoveredLifecycleNonce: String,
        recoveredGeneration: UInt64,
        conflictSetID: GaryxPayloadConflictSetID,
        incompleteAttachmentFeedbackID: GaryxFeedbackID
    ) -> GaryxDeliveryDraftRecoveryPlan? {
        guard let delivery = snapshot.deliveries[deliveryID],
              delivery.phase == .notDispatched,
              delivery.userDisposition == .none,
              !isOwnedByCreate(delivery, snapshot: snapshot) else {
            return nil
        }
        return GaryxDeliveryDraftRecoveryPlanner.plan(
            snapshot: snapshot,
            deliveryID: deliveryID,
            recoveredEntryID: recoveredEntryID,
            recoveredLifecycleNonce: recoveredLifecycleNonce,
            recoveredGeneration: recoveredGeneration,
            conflictSetID: conflictSetID,
            allowingUndispatched: true,
            incompleteAttachmentFeedbackID: incompleteAttachmentFeedbackID
        )
    }

    /// Automatic recovery identities are derived from the durable delivery so
    /// a crash before or after the recovery transaction can retry the same
    /// logical exit without creating duplicate deferred payloads or feedback.
    public static func automaticPlan(
        snapshot: GaryxComposerDurabilitySnapshot,
        deliveryID: GaryxDeliveryRecordID,
        recoveredGeneration: UInt64
    ) -> GaryxDeliveryDraftRecoveryPlan? {
        let component = deliveryID.rawValue
        return plan(
            snapshot: snapshot,
            deliveryID: deliveryID,
            recoveredEntryID: .init(rawValue: "undispatched-recovery-\(component)"),
            recoveredLifecycleNonce: "undispatched-recovery-token-\(component)",
            recoveredGeneration: recoveredGeneration,
            conflictSetID: .init(rawValue: "undispatched-recovery-\(component)"),
            incompleteAttachmentFeedbackID: .init(
                rawValue: "undispatched-recovery-attachments-\(component)"
            )
        )
    }

    public static func isOwnedByCreate(
        _ delivery: GaryxDeliveryRecord,
        snapshot: GaryxComposerDurabilitySnapshot
    ) -> Bool {
        snapshot.createDeliveries.values.contains {
            $0.scope == delivery.scope
                && $0.entryID == delivery.entryID
                && $0.createIntentID == delivery.correlationID
                && !$0.isTerminalCorrelation
        }
    }
}

public struct GaryxDeliveryDuplicateResendPlan: Equatable, Sendable {
    public let envelope: GaryxDeliveryEnvelope
    public let newDeliveryID: GaryxDeliveryRecordID
    public let transaction: GaryxComposerDurabilityTransaction
}

public enum GaryxDeliveryDuplicateResendPlanner {
    public static func plan(
        snapshot: GaryxComposerDurabilitySnapshot,
        deliveryID: GaryxDeliveryRecordID,
        newDeliveryID: GaryxDeliveryRecordID,
        newClientIntentID: String,
        allowingUndispatchedCreate: Bool = false
    ) -> GaryxDeliveryDuplicateResendPlan? {
        guard snapshot.deliveries[newDeliveryID] == nil,
              var original = snapshot.deliveries[deliveryID],
              var entry = snapshot.payloadStore.entry(
                original.entryID,
                scope: original.scope
              ),
              entry.lifecycle.phase == .active,
              let envelope = original.resendAsDuplicate(
                newRecordID: newDeliveryID,
                newClientIntentID: newClientIntentID,
                allowingUndispatched: allowingUndispatchedCreate
              ) else {
            return nil
        }
        let duplicate = GaryxDeliveryRecord(
            id: newDeliveryID,
            scope: original.scope,
            entryID: original.entryID,
            reservationID: original.reservationID,
            correlationID: newClientIntentID,
            envelope: envelope
        )
        entry.removeDeliveryReference(deliveryID)
        entry.addDeliveryReference(newDeliveryID)
        return GaryxDeliveryDuplicateResendPlan(
            envelope: envelope,
            newDeliveryID: newDeliveryID,
            transaction: GaryxComposerDurabilityTransaction(
                expectedRevision: snapshot.revision,
                label: "supersede ambiguous delivery with duplicate-risk copy",
                mutations: [
                    .upsertEntry(entry),
                    .upsertDelivery(original),
                    .upsertDelivery(duplicate),
                ]
            )
        )
    }
}

// MARK: - Atomic multi-stage create exits

public struct GaryxCreateDraftRecoveryPlan: Equatable, Sendable {
    public let envelope: GaryxDeliveryEnvelope
    public let deliveryID: GaryxDeliveryRecordID
    public let placement: GaryxRecoveredDraftPlacement
    public let transaction: GaryxComposerDurabilityTransaction
}

public enum GaryxCreateDraftRecoveryPlanner {
    /// The create response and its message envelope settle together. A
    /// not-dispatched message is safe to restore, while an attempted chat-start
    /// uses the same non-overwriting automatic placement as an ambiguous
    /// delivery.
    public static func plan(
        snapshot: GaryxComposerDurabilitySnapshot,
        key: GaryxCreateDeliveryKey,
        recoveredEntryID: GaryxComposerPayloadEntryID,
        recoveredLifecycleNonce: String,
        recoveredGeneration: UInt64,
        conflictSetID: GaryxPayloadConflictSetID
    ) -> GaryxCreateDraftRecoveryPlan? {
        guard var create = snapshot.createDeliveries[key],
              create.phase == .ambiguous,
              create.userDisposition == .none,
              let entryID = create.entryID else {
            return nil
        }
        let matches = snapshot.deliveries.values.filter {
            $0.scope == key.scope
                && $0.entryID == entryID
                && $0.correlationID == key.createIntentID
                && ($0.phase == .notDispatched || $0.phase == .ambiguous)
                && $0.userDisposition == .none
        }
        guard matches.count == 1,
              let delivery = matches.first,
              create.restoreToDraft(),
              let deliveryPlan = GaryxDeliveryDraftRecoveryPlanner.plan(
                snapshot: snapshot,
                deliveryID: delivery.id,
                recoveredEntryID: recoveredEntryID,
                recoveredLifecycleNonce: recoveredLifecycleNonce,
                recoveredGeneration: recoveredGeneration,
                conflictSetID: conflictSetID,
                allowingUndispatched: true
              ) else {
            return nil
        }
        return GaryxCreateDraftRecoveryPlan(
            envelope: deliveryPlan.envelope,
            deliveryID: delivery.id,
            placement: deliveryPlan.placement,
            transaction: .init(
                expectedRevision: snapshot.revision,
                label: "restore ambiguous create with automatic payload placement",
                mutations: deliveryPlan.transaction.mutations + [.upsertCreateDelivery(create)]
            )
        )
    }
}

public struct GaryxCreateDuplicateRebuildPlan: Equatable, Sendable {
    public let envelope: GaryxDeliveryEnvelope
    public let originalDeliveryID: GaryxDeliveryRecordID
    public let newDeliveryID: GaryxDeliveryRecordID
    public let newCreateKey: GaryxCreateDeliveryKey?
    public let ambiguousAfter: GaryxCreateDeliveryPhase?
    public let transaction: GaryxComposerDurabilityTransaction
}

public enum GaryxCreateDuplicateRebuildPlanner {
    /// Rebuild is intentionally duplicate-risk: both the create intent and the
    /// message intent change, and the original correlation remains available
    /// for late evidence without mutating the new copy.
    public static func plan(
        snapshot: GaryxComposerDurabilitySnapshot,
        key: GaryxCreateDeliveryKey,
        newCreateIntentID: String,
        newDeliveryID: GaryxDeliveryRecordID
    ) -> GaryxCreateDuplicateRebuildPlan? {
        guard var create = snapshot.createDeliveries[key],
              create.phase == .ambiguous,
              create.userDisposition == .none,
              let entryID = create.entryID else {
            return nil
        }
        let matches = snapshot.deliveries.values.filter {
            $0.scope == key.scope
                && $0.entryID == entryID
                && $0.correlationID == key.createIntentID
                && ($0.phase == .notDispatched || $0.phase == .ambiguous)
                && $0.userDisposition == .none
        }
        guard matches.count == 1,
              let delivery = matches.first,
              let duplicateRiskCreate = create.rebuildWithDuplicateRisk(
                newCreateIntentID: newCreateIntentID
              ),
              let deliveryPlan = GaryxDeliveryDuplicateResendPlanner.plan(
                snapshot: snapshot,
                deliveryID: delivery.id,
                newDeliveryID: newDeliveryID,
                newClientIntentID: newCreateIntentID,
                allowingUndispatchedCreate: true
              ) else {
            return nil
        }
        // Only create-response loss lacks a known thread. Once threadID is
        // durable, the duplicate-risk copy reuses that known destination and
        // must not leave an orphan createPending record behind.
        let nextCreate = create.threadID == nil ? duplicateRiskCreate : nil
        var createMutations: [GaryxComposerDurabilityMutation] = [
            .upsertCreateDelivery(create),
        ]
        if let nextCreate {
            createMutations.append(.upsertCreateDelivery(nextCreate))
        }
        return GaryxCreateDuplicateRebuildPlan(
            envelope: deliveryPlan.envelope,
            originalDeliveryID: delivery.id,
            newDeliveryID: newDeliveryID,
            newCreateKey: nextCreate?.key,
            ambiguousAfter: create.ambiguousAfter,
            transaction: .init(
                expectedRevision: snapshot.revision,
                label: "supersede ambiguous create with duplicate-risk copy",
                mutations: deliveryPlan.transaction.mutations + createMutations
            )
        )
    }
}

public struct GaryxDeferredDraftAdoptionCandidate: Equatable, Sendable {
    public let conflictSetID: GaryxPayloadConflictSetID
    public let hostEntryID: GaryxComposerPayloadEntryID
    public let recoveredEntryID: GaryxComposerPayloadEntryID

    public init(
        conflictSetID: GaryxPayloadConflictSetID,
        hostEntryID: GaryxComposerPayloadEntryID,
        recoveredEntryID: GaryxComposerPayloadEntryID
    ) {
        self.conflictSetID = conflictSetID
        self.hostEntryID = hostEntryID
        self.recoveredEntryID = recoveredEntryID
    }
}

public struct GaryxDeferredDraftAdoptionPlan: Equatable, Sendable {
    public let transaction: GaryxComposerDurabilityTransaction
}

public enum GaryxDeferredDraftAdoptionPlanner {
    /// Finds the earliest deferred recovered payload whose host is truly empty.
    /// A meaningful host is never modified; its deferred entries remain rooted
    /// by the durable candidate set until a later send or relaunch can adopt
    /// them safely.
    public static func candidate(
        snapshot: GaryxComposerDurabilitySnapshot,
        hostEntryID: GaryxComposerPayloadEntryID? = nil
    ) -> GaryxDeferredDraftAdoptionCandidate? {
        snapshot.conflicts.values.compactMap { conflict -> (
            candidate: GaryxDeferredDraftAdoptionCandidate,
            generation: UInt64
        )? in
            guard conflict.candidates.count == 2,
                  let host = conflict.candidates.first(where: { $0.label == "Current draft" }),
                  let recovered = conflict.candidates.first(where: { $0.label == "Recovered send" }),
                  hostEntryID == nil || host.entryID == hostEntryID,
                  let hostEntry = snapshot.payloadStore.entry(host.entryID, scope: conflict.scope),
                  !hostEntry.hasMeaningfulCurrentPayload,
                  let recoveredEntry = snapshot.payloadStore.entry(
                      recovered.entryID,
                      scope: conflict.scope
                  ) else {
                return nil
            }
            return (
                GaryxDeferredDraftAdoptionCandidate(
                    conflictSetID: conflict.id,
                    hostEntryID: host.entryID,
                    recoveredEntryID: recovered.entryID
                ),
                recoveredEntry.currentGeneration
            )
        }
        .sorted {
            if $0.generation != $1.generation {
                return $0.generation < $1.generation
            }
            return $0.candidate.conflictSetID.rawValue
                < $1.candidate.conflictSetID.rawValue
        }
        .first?.candidate
    }

    public static func plan(
        snapshot: GaryxComposerDurabilitySnapshot,
        candidate: GaryxDeferredDraftAdoptionCandidate,
        replacementGeneration: UInt64
    ) -> GaryxDeferredDraftAdoptionPlan? {
        guard replacementGeneration > snapshot.generationClaimFloor,
              replacementGeneration <= snapshot.generationHighWatermark,
              !snapshot.claimedGenerations.contains(replacementGeneration),
              let conflict = snapshot.conflicts[candidate.conflictSetID],
              conflict.candidates.count == 2,
              conflict.candidates.contains(where: {
                  $0.entryID == candidate.hostEntryID && $0.label == "Current draft"
              }),
              conflict.candidates.contains(where: {
                  $0.entryID == candidate.recoveredEntryID && $0.label == "Recovered send"
              }),
              var host = snapshot.payloadStore.entry(
                candidate.hostEntryID,
                scope: conflict.scope
              ),
              !host.hasMeaningfulCurrentPayload,
              let recovered = snapshot.payloadStore.entry(
                candidate.recoveredEntryID,
                scope: conflict.scope
              ),
              host.replaceCurrentPayload(
                text: recovered.currentText,
                attachments: Array(recovered.attachments.values),
                generation: replacementGeneration
              ) else {
            return nil
        }
        return GaryxDeferredDraftAdoptionPlan(
            transaction: .init(
                expectedRevision: snapshot.revision,
                label: "adopt deferred recovered payload into empty composer",
                mutations: [
                    .claimGeneration(replacementGeneration),
                    .upsertEntry(host),
                    .removeEntry(scope: conflict.scope, entryID: candidate.recoveredEntryID),
                    .removeConflict(candidate.conflictSetID),
                ]
            )
        )
    }
}

// MARK: - Evidence-only ingress

public struct GaryxDeliveryEvidencePlan: Equatable, Sendable {
    public let disposition: GaryxDeliveryEvidenceIngressDisposition
    public let transaction: GaryxComposerDurabilityTransaction?
}

public enum GaryxDeliveryEvidencePlanner {
    public static func plan(
        snapshot: GaryxComposerDurabilitySnapshot,
        correlationID: String,
        authenticatedScope: GaryxGatewayScope
    ) -> GaryxDeliveryEvidencePlan {
        var records = snapshot.deliveries
        let disposition = GaryxDeliveryEvidenceIngress.acknowledge(
            correlationID: correlationID,
            authenticatedScope: authenticatedScope,
            records: &records
        )
        guard case .updated(let id) = disposition,
              let updated = records[id],
              updated != snapshot.deliveries[id] else {
            return GaryxDeliveryEvidencePlan(disposition: disposition, transaction: nil)
        }
        var mutations: [GaryxComposerDurabilityMutation] = [.upsertDelivery(updated)]
        if updated.phase.isTerminalOrEvidence,
           var entry = snapshot.payloadStore.entry(updated.entryID, scope: updated.scope) {
            entry.removeDeliveryReference(id)
            mutations.insert(.upsertEntry(entry), at: 0)
        }
        return GaryxDeliveryEvidencePlan(
            disposition: disposition,
            transaction: .init(
                expectedRevision: snapshot.revision,
                label: "ingest authenticated delivery evidence",
                mutations: mutations
            )
        )
    }
}

// MARK: - Scope settlement

public struct GaryxGatewayScopeSettlementPlan: Equatable, Sendable {
    public let transaction: GaryxComposerDurabilityTransaction
}

public enum GaryxGatewayScopeSettlementPlanner {
    /// Revocation and every DeliveryRecord CAS publish together. Payload and
    /// file cleanup can then converge through the existing discard recovery,
    /// while the durable watermark already rejects all domain resurrection.
    public static func revoke(
        snapshot: GaryxComposerDurabilitySnapshot,
        scope: GaryxGatewayScope
    ) -> GaryxGatewayScopeSettlementPlan? {
        var registry = snapshot.scopeRegistry
        if registry.lifecycle(of: scope) == .revoked,
           scope.epoch <= (registry.revokedThroughEpoch[scope.identity] ?? 0) {
            return nil
        }
        if registry.lifecycle(of: scope) == .revoked {
            _ = registry.switchActive(to: scope)
        }
        _ = registry.revoke(scope)
        var mutations: [GaryxComposerDurabilityMutation] = [.replaceScopeRegistry(registry)]

        let scopedDeliveries = snapshot.deliveries.values
            .filter { $0.scope == scope }
            .sorted { $0.id.rawValue < $1.id.rawValue }
        for var delivery in scopedDeliveries {
            delivery.settleForScopeRevoke()
            mutations.append(.upsertDelivery(delivery))
        }
        var archivedFeedbackIDs = Set<GaryxFeedbackID>()
        for var feedback in snapshot.feedback.values
            .filter({ $0.scope == scope })
            .sorted(by: { $0.id.rawValue < $1.id.rawValue }) {
            feedback.archive()
            archivedFeedbackIDs.insert(feedback.id)
            mutations.append(.upsertFeedback(feedback))
        }
        let scopedEntries = snapshot.payloadStore.entriesByScope[scope].map {
            Array($0.values)
        } ?? []
        for var entry in scopedEntries.sorted(by: { $0.id.rawValue < $1.id.rawValue }) {
            for id in entry.feedbackReferences where archivedFeedbackIDs.contains(id) {
                entry.removeFeedbackReference(id)
            }
            mutations.append(.upsertEntry(entry))
        }
        for var create in snapshot.createDeliveries.values
            .filter({ $0.scope == scope && !$0.isTerminalCorrelation })
            .sorted(by: { $0.createIntentID < $1.createIntentID }) {
            create.settleForScopeRevoke()
            mutations.append(.upsertCreateDelivery(create))
        }
        return GaryxGatewayScopeSettlementPlan(
            transaction: .init(
                expectedRevision: snapshot.revision,
                label: "revoke gateway scope and settle delivery evidence",
                mutations: mutations
            )
        )
    }
}

// MARK: - Durable composer notices

public enum GaryxComposerDurableNoticeKind: String, Codable, Sendable {
    case ambiguousDelivery
    case ambiguousCreate
    case feedback
}

public enum GaryxComposerDurableNoticeAction: Equatable, Codable, Sendable {
    case restoreDelivery(GaryxDeliveryRecordID)
    case resendDeliveryCopy(GaryxDeliveryRecordID)
    case restoreCreate(GaryxCreateDeliveryKey)
    case rebuildCreateCopy(GaryxCreateDeliveryKey)
    case acknowledgeFeedback(GaryxFeedbackID)
    case retryUpload(GaryxFeedbackID)
    case removeUpload(GaryxFeedbackID)
}

public struct GaryxComposerDurableNotice: Equatable, Codable, Identifiable, Sendable {
    public let id: String
    public let kind: GaryxComposerDurableNoticeKind
    public let title: String
    public let detail: String
    public let actions: [GaryxComposerDurableNoticeAction]

    public init(
        id: String,
        kind: GaryxComposerDurableNoticeKind,
        title: String,
        detail: String,
        actions: [GaryxComposerDurableNoticeAction]
    ) {
        self.id = id
        self.kind = kind
        self.title = title
        self.detail = detail
        self.actions = actions
    }
}

public enum GaryxComposerDurableNoticeProjector {
    public static func project(
        snapshot: GaryxComposerDurabilitySnapshot,
        hostEntryID: GaryxComposerPayloadEntryID,
        hasInteractionOwner: Bool
    ) -> [GaryxComposerDurableNotice] {
        guard hasInteractionOwner,
              let entry = snapshot.payloadStore.entriesByScope.values
                .lazy
                .compactMap({ $0[hostEntryID] })
                .first else {
            return []
        }
        var notices: [GaryxComposerDurableNotice] = []
        let ambiguousCreateIntentIDs = Set(
            snapshot.createDeliveries.values.lazy.filter {
                $0.entryID == hostEntryID
                    && $0.scope == entry.scope
                    && $0.phase == .ambiguous
                    && $0.userDisposition == .none
            }.map(\.createIntentID)
        )
        for delivery in snapshot.deliveries.values
            .filter({
                $0.entryID == hostEntryID
                    && $0.scope == entry.scope
                    && $0.phase == .ambiguous
                    && $0.userDisposition == .none
                    && !ambiguousCreateIntentIDs.contains($0.correlationID)
            })
            .sorted(by: { $0.id.rawValue < $1.id.rawValue }) {
            notices.append(
                GaryxComposerDurableNotice(
                    id: "delivery:\(delivery.id.rawValue)",
                    kind: .ambiguousDelivery,
                    title: "Send status unknown",
                    detail: "The gateway may have accepted this message. Resending can create a duplicate.",
                    actions: [
                        .restoreDelivery(delivery.id),
                        .resendDeliveryCopy(delivery.id),
                    ]
                )
            )
        }
        for create in snapshot.createDeliveries.values
            .filter({
                $0.entryID == hostEntryID
                    && $0.scope == entry.scope
                    && $0.phase == .ambiguous
                    && $0.userDisposition == .none
            })
            .sorted(by: { $0.createIntentID < $1.createIntentID }) {
            let chatWasAttempted = create.ambiguousAfter == .chatStartAttempted
            notices.append(
                GaryxComposerDurableNotice(
                    id: "create:\(create.createIntentID)",
                    kind: .ambiguousCreate,
                    title: chatWasAttempted
                        ? "Send status unknown"
                        : "Conversation creation status unknown",
                    detail: chatWasAttempted
                        ? "The gateway may have accepted this message. Resending can create a duplicate."
                        : "The conversation may already exist. Rebuilding can create another conversation.",
                    actions: [
                        .restoreCreate(create.key),
                        .rebuildCreateCopy(create.key),
                    ]
                )
            )
        }
        for feedback in snapshot.feedback.values
            .filter({
                $0.scope == entry.scope
                    && $0.entryID == hostEntryID
                    && ($0.phase == .pending || $0.phase == .presented)
            })
            .sorted(by: { $0.id.rawValue < $1.id.rawValue }) {
            let content = feedbackContent(feedback)
            notices.append(
                GaryxComposerDurableNotice(
                    id: "feedback:\(feedback.id.rawValue)",
                    kind: .feedback,
                    title: content.title,
                    detail: content.detail,
                    actions: content.actions
                )
            )
        }
        return notices
    }

    private static func feedbackContent(
        _ feedback: GaryxOperationFeedback
    ) -> (title: String, detail: String, actions: [GaryxComposerDurableNoticeAction]) {
        switch feedback.kind {
        case .deliveryBackpressure:
            return (
                "Too many sends awaiting confirmation",
                "This draft was kept. Resolve an unknown send before trying again.",
                [.acknowledgeFeedback(feedback.id)]
            )
        case .quotaExceeded:
            return (
                "Attachment storage is full",
                "Remove an attachment and try again.",
                [.acknowledgeFeedback(feedback.id)]
            )
        case .uploadRetryable:
            return (
                "Upload did not finish",
                "Retry the upload or remove this attachment.",
                [.retryUpload(feedback.id), .removeUpload(feedback.id)]
            )
        case .uploadTerminal:
            return (
                "Attachment could not be uploaded",
                "Remove it or choose a replacement.",
                [.removeUpload(feedback.id)]
            )
        case .deliveryAttachmentRecoveryIncomplete:
            return (
                "Some attachments could not be restored",
                "The unsent message text was recovered. Reattach the missing files before sending.",
                [.acknowledgeFeedback(feedback.id)]
            )
        }
    }
}

public enum GaryxFeedbackPresentationPlanner {
    public static func plan(
        snapshot: GaryxComposerDurabilitySnapshot,
        hostEntryID: GaryxComposerPayloadEntryID,
        hasInteractionOwner: Bool
    ) -> GaryxComposerDurabilityTransaction? {
        var mutations: [GaryxComposerDurabilityMutation] = []
        for var feedback in snapshot.feedback.values
            .filter({ $0.entryID == hostEntryID && $0.phase == .pending })
            .sorted(by: { $0.id.rawValue < $1.id.rawValue }) {
            guard feedback.present(
                hostEntryID: hostEntryID,
                hasInteractionOwner: hasInteractionOwner
            ) else { continue }
            mutations.append(.upsertFeedback(feedback))
        }
        guard !mutations.isEmpty else { return nil }
        return .init(
            expectedRevision: snapshot.revision,
            label: "present durable composer feedback to interaction owner",
            mutations: mutations
        )
    }
}

public enum GaryxFeedbackAcknowledgementPlanner {
    public static func plan(
        snapshot: GaryxComposerDurabilitySnapshot,
        feedbackID: GaryxFeedbackID,
        hostEntryID: GaryxComposerPayloadEntryID
    ) -> GaryxComposerDurabilityTransaction? {
        guard var feedback = snapshot.feedback[feedbackID],
              feedback.entryID == hostEntryID,
              feedback.kind == .deliveryBackpressure
                || feedback.kind == .quotaExceeded
                || feedback.kind == .deliveryAttachmentRecoveryIncomplete,
              var entry = snapshot.payloadStore.entry(hostEntryID, scope: feedback.scope),
              !feedback.isTerminal else {
            return nil
        }
        feedback.acknowledge()
        entry.removeFeedbackReference(feedbackID)
        return .init(
            expectedRevision: snapshot.revision,
            label: "acknowledge durable composer feedback",
            mutations: [.upsertEntry(entry), .upsertFeedback(feedback)]
        )
    }
}

public enum GaryxDeliveryBackpressurePlanner {
    public static func plan(
        snapshot: GaryxComposerDurabilitySnapshot,
        entryID: GaryxComposerPayloadEntryID,
        envelopeBytes: Int,
        feedbackID: GaryxFeedbackID
    ) -> GaryxComposerDurabilityTransaction? {
        guard let scopedEntry = snapshot.payloadStore.entriesByScope.first(where: {
            $0.value[entryID] != nil
        }), var entry = scopedEntry.value[entryID] else {
            return nil
        }
        let quota = GaryxDeliveryQuota(rebuilding: Array(snapshot.deliveries.values))
        guard !quota.canSeal(scope: entry.scope, envelopeBytes: envelopeBytes) else {
            return nil
        }
        if snapshot.feedback.values.contains(where: {
            $0.entryID == entryID
                && $0.scope == entry.scope
                && $0.kind == .deliveryBackpressure
                && !$0.isTerminal
        }) {
            return nil
        }
        let feedback = GaryxOperationFeedback(
            id: feedbackID,
            scope: entry.scope,
            entryID: entryID,
            operationID: nil,
            kind: .deliveryBackpressure
        )
        entry.addFeedbackReference(feedbackID)
        return .init(
            expectedRevision: snapshot.revision,
            label: "retain draft under delivery backpressure",
            mutations: [.upsertEntry(entry), .upsertFeedback(feedback)]
        )
    }
}
