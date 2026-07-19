import Foundation

// MARK: - Atomic ambiguous-delivery exits

public struct GaryxDeliveryDraftRecoveryPlan: Equatable, Sendable {
    public let envelope: GaryxDeliveryEnvelope
    public let recoveredEntryID: GaryxComposerPayloadEntryID
    public let conflictSetID: GaryxPayloadConflictSetID
    public let transaction: GaryxComposerDurabilityTransaction
}

public enum GaryxDeliveryDraftRecoveryPlanner {
    /// Restores an ambiguous envelope as a separate conflict candidate. It
    /// never writes into the active follow-up Entry; that requires a later,
    /// explicit conflict-resolution transaction.
    public static func plan(
        snapshot: GaryxComposerDurabilitySnapshot,
        deliveryID: GaryxDeliveryRecordID,
        recoveredEntryID: GaryxComposerPayloadEntryID,
        recoveredLifecycleNonce: String,
        recoveredGeneration: UInt64,
        conflictSetID: GaryxPayloadConflictSetID
    ) -> GaryxDeliveryDraftRecoveryPlan? {
        guard recoveredGeneration > snapshot.generationClaimFloor,
              recoveredGeneration <= snapshot.generationHighWatermark,
              !snapshot.claimedGenerations.contains(recoveredGeneration),
              var record = snapshot.deliveries[deliveryID],
              let originalEnvelope = record.envelope,
              originalEnvelope.attachmentIDs.isEmpty
                || Set(originalEnvelope.attachments.map(\.id))
                    == Set(originalEnvelope.attachmentIDs),
              var hostEntry = snapshot.payloadStore.entry(
                record.entryID,
                scope: record.scope
              ),
              hostEntry.lifecycle.phase == .active,
              snapshot.payloadStore.entry(recoveredEntryID, scope: record.scope) == nil,
              !recoveredLifecycleNonce.isEmpty else {
            return nil
        }

        var conflict = snapshot.conflicts[conflictSetID]
            ?? GaryxPayloadConflictSet(id: conflictSetID, scope: record.scope)
        guard conflict.scope == record.scope,
              conflict.admitCandidate(
                GaryxPayloadConflictCandidate(
                    entryID: hostEntry.id,
                    label: "Current draft"
                ),
                membershipDurabilityAvailable: true
              ) else {
            return nil
        }
        let recoveredCandidate = GaryxPayloadConflictCandidate(
            entryID: recoveredEntryID,
            label: "Recovered send"
        )
        guard case .restored(let envelope) = GaryxDeliveryDraftRecoveryReducer.restore(
            record: &record,
            conflictSet: &conflict,
            candidate: recoveredCandidate,
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
        for attachment in envelope.attachments {
            recoveredEntry.addAttachment(
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
            )
        }
        hostEntry.removeDeliveryReference(deliveryID)
        return GaryxDeliveryDraftRecoveryPlan(
            envelope: envelope,
            recoveredEntryID: recoveredEntryID,
            conflictSetID: conflictSetID,
            transaction: GaryxComposerDurabilityTransaction(
                expectedRevision: snapshot.revision,
                label: "restore ambiguous delivery through payload conflict",
                mutations: [
                    .claimGeneration(recoveredGeneration),
                    .upsertEntry(hostEntry),
                    .upsertEntry(recoveredEntry),
                    .upsertConflict(conflict),
                    .upsertDelivery(record),
                ]
            )
        )
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
        newClientIntentID: String
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
                newClientIntentID: newClientIntentID
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

public struct GaryxRecoveredDraftResolutionPlan: Equatable, Sendable {
    public let transaction: GaryxComposerDurabilityTransaction
}

public enum GaryxRecoveredDraftResolutionPlanner {
    public static func keepCurrentDraft(
        snapshot: GaryxComposerDurabilitySnapshot,
        conflictSetID: GaryxPayloadConflictSetID,
        hostEntryID: GaryxComposerPayloadEntryID,
        recoveredEntryID: GaryxComposerPayloadEntryID
    ) -> GaryxRecoveredDraftResolutionPlan? {
        guard let conflict = snapshot.conflicts[conflictSetID],
              conflict.pendingDecision,
              conflict.candidates.contains(where: { $0.entryID == hostEntryID }),
              conflict.candidates.contains(where: { $0.entryID == recoveredEntryID }),
              let recovered = snapshot.payloadStore.entry(
                recoveredEntryID,
                scope: conflict.scope
              ),
              recovered.isReclaimable == false else {
            return nil
        }
        return GaryxRecoveredDraftResolutionPlan(
            transaction: .init(
                expectedRevision: snapshot.revision,
                label: "keep current draft over recovered delivery",
                mutations: [
                    .removeEntry(scope: conflict.scope, entryID: recoveredEntryID),
                    .removeConflict(conflictSetID),
                ]
            )
        )
    }

    public static func useRecoveredDraft(
        snapshot: GaryxComposerDurabilitySnapshot,
        conflictSetID: GaryxPayloadConflictSetID,
        hostEntryID: GaryxComposerPayloadEntryID,
        recoveredEntryID: GaryxComposerPayloadEntryID,
        replacementGeneration: UInt64
    ) -> GaryxRecoveredDraftResolutionPlan? {
        guard replacementGeneration > snapshot.generationClaimFloor,
              replacementGeneration <= snapshot.generationHighWatermark,
              !snapshot.claimedGenerations.contains(replacementGeneration),
              let conflict = snapshot.conflicts[conflictSetID],
              conflict.pendingDecision,
              conflict.candidates.contains(where: { $0.entryID == hostEntryID }),
              conflict.candidates.contains(where: { $0.entryID == recoveredEntryID }),
              var host = snapshot.payloadStore.entry(hostEntryID, scope: conflict.scope),
              let recovered = snapshot.payloadStore.entry(
                recoveredEntryID,
                scope: conflict.scope
              ),
              host.replaceCurrentPayload(
                text: recovered.currentText,
                attachments: Array(recovered.attachments.values),
                generation: replacementGeneration
              ) else {
            return nil
        }
        return GaryxRecoveredDraftResolutionPlan(
            transaction: .init(
                expectedRevision: snapshot.revision,
                label: "replace current draft with recovered delivery",
                mutations: [
                    .claimGeneration(replacementGeneration),
                    .upsertEntry(host),
                    .removeEntry(scope: conflict.scope, entryID: recoveredEntryID),
                    .removeConflict(conflictSetID),
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
    case payloadConflict
    case feedback
}

public enum GaryxComposerDurableNoticeAction: Equatable, Codable, Sendable {
    case restoreDelivery(GaryxDeliveryRecordID)
    case resendDeliveryCopy(GaryxDeliveryRecordID)
    case restoreCreate(GaryxCreateDeliveryKey)
    case rebuildCreateCopy(GaryxCreateDeliveryKey)
    case useRecoveredDraft(GaryxPayloadConflictSetID, GaryxComposerPayloadEntryID)
    case keepCurrentDraft(GaryxPayloadConflictSetID, GaryxComposerPayloadEntryID)
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
        for delivery in snapshot.deliveries.values
            .filter({
                $0.entryID == hostEntryID
                    && $0.scope == entry.scope
                    && $0.phase == .ambiguous
                    && $0.userDisposition == .none
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
            notices.append(
                GaryxComposerDurableNotice(
                    id: "create:\(create.createIntentID)",
                    kind: .ambiguousCreate,
                    title: "Conversation creation status unknown",
                    detail: "The conversation may already exist. Rebuilding can create another conversation.",
                    actions: [
                        .restoreCreate(create.key),
                        .rebuildCreateCopy(create.key),
                    ]
                )
            )
        }
        for conflict in snapshot.conflicts.values
            .filter({
                $0.scope == entry.scope
                    && $0.pendingDecision
                    && $0.candidates.contains(where: { $0.entryID == hostEntryID })
            })
            .sorted(by: { $0.id.rawValue < $1.id.rawValue }) {
            guard let recovered = conflict.candidates.first(where: { $0.entryID != hostEntryID }) else {
                continue
            }
            notices.append(
                GaryxComposerDurableNotice(
                    id: "conflict:\(conflict.id.rawValue)",
                    kind: .payloadConflict,
                    title: "Recovered message is ready",
                    detail: "Choose which draft should remain in the composer.",
                    actions: [
                        .useRecoveredDraft(conflict.id, recovered.entryID),
                        .keepCurrentDraft(conflict.id, recovered.entryID),
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
              feedback.kind == .deliveryBackpressure || feedback.kind == .quotaExceeded,
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
