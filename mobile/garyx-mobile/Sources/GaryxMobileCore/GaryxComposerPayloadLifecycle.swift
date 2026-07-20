import Foundation

// MARK: - Payload child identities

public struct GaryxAttachmentID: RawRepresentable, Hashable, Codable, Sendable {
    public let rawValue: String
    public init(rawValue: String) {
        precondition(!rawValue.isEmpty)
        self.rawValue = rawValue
    }
}

public struct GaryxStagedAssetID: RawRepresentable, Hashable, Codable, Sendable {
    public let rawValue: String
    public init(rawValue: String) {
        precondition(!rawValue.isEmpty)
        self.rawValue = rawValue
    }
}

public struct GaryxOperationID: RawRepresentable, Hashable, Codable, Sendable {
    public let rawValue: String
    public init(rawValue: String) {
        precondition(!rawValue.isEmpty)
        self.rawValue = rawValue
    }
}

public struct GaryxReplacementID: RawRepresentable, Hashable, Codable, Sendable {
    public let rawValue: String
    public init(rawValue: String) {
        precondition(!rawValue.isEmpty)
        self.rawValue = rawValue
    }
}

public struct GaryxFeedbackID: RawRepresentable, Hashable, Codable, Sendable {
    public let rawValue: String
    public init(rawValue: String) {
        precondition(!rawValue.isEmpty)
        self.rawValue = rawValue
    }
}

public struct GaryxPayloadConflictSetID: RawRepresentable, Hashable, Codable, Sendable {
    public let rawValue: String
    public init(rawValue: String) {
        precondition(!rawValue.isEmpty)
        self.rawValue = rawValue
    }
}

public struct GaryxAttachmentLineageID: RawRepresentable, Hashable, Codable, Sendable {
    public let rawValue: String
    public init(rawValue: String) {
        precondition(!rawValue.isEmpty)
        self.rawValue = rawValue
    }
}

public struct GaryxDeliveryRecordID: RawRepresentable, Hashable, Codable, Sendable {
    public let rawValue: String
    public init(rawValue: String) {
        precondition(!rawValue.isEmpty)
        self.rawValue = rawValue
    }
}

public enum GaryxOperationBranch: String, Codable, Sendable {
    case envelope
    case followup
}

public struct GaryxComposerAttachment: Equatable, Codable, Sendable {
    public let id: GaryxAttachmentID
    public let stagedAssetID: GaryxStagedAssetID
    public let generation: UInt64
    public let byteCount: Int
    public let kind: String?
    public let name: String?
    public let mediaType: String?
    public let uploadedPath: String?
    public let previewDataURL: String?

    public init(
        id: GaryxAttachmentID,
        stagedAssetID: GaryxStagedAssetID,
        generation: UInt64,
        byteCount: Int,
        kind: String? = nil,
        name: String? = nil,
        mediaType: String? = nil,
        uploadedPath: String? = nil,
        previewDataURL: String? = nil
    ) {
        precondition(byteCount >= 0)
        self.id = id
        self.stagedAssetID = stagedAssetID
        self.generation = generation
        self.byteCount = byteCount
        self.kind = kind
        self.name = name
        self.mediaType = mediaType
        self.uploadedPath = uploadedPath
        self.previewDataURL = previewDataURL
    }

    public func recordingUpload(
        kind: String,
        name: String,
        mediaType: String,
        path: String
    ) -> Self {
        Self(
            id: id,
            stagedAssetID: stagedAssetID,
            generation: generation,
            byteCount: byteCount,
            kind: kind,
            name: name,
            mediaType: mediaType,
            uploadedPath: path,
            previewDataURL: previewDataURL
        )
    }
}

// MARK: - Payload entry and lifecycle

public struct GaryxPayloadLifecycleRecord: Equatable, Codable, Sendable {
    public let token: GaryxPayloadLifecycleToken
    public private(set) var revision: UInt64
    public private(set) var phase: GaryxPayloadLifecyclePhase
    public private(set) var discardRevision: UInt64?

    public init(token: GaryxPayloadLifecycleToken, revision: UInt64 = 1) {
        self.token = token
        self.revision = revision
        phase = .active
        discardRevision = nil
    }

    public var snapshot: GaryxPayloadLifecycleSnapshot {
        GaryxPayloadLifecycleSnapshot(token: token, revision: revision, phase: phase)
    }

    @discardableResult
    public mutating func beginDiscard(discardRevision: UInt64) -> Bool {
        guard phase == .active else { return false }
        revision &+= 1
        phase = .discarding
        self.discardRevision = discardRevision
        return true
    }

    @discardableResult
    public mutating func finishDiscard(
        reservationSettled: Bool,
        descendantsEmpty: Bool,
        deliveriesSettled: Bool
    ) -> Bool {
        guard phase == .discarding,
              reservationSettled,
              descendantsEmpty,
              deliveriesSettled else {
            return false
        }
        phase = .discarded
        return true
    }
}

public struct GaryxComposerPayloadEntry: Equatable, Codable, Sendable {
    public let id: GaryxComposerPayloadEntryID
    public let scope: GaryxGatewayScope
    public private(set) var destination: GaryxComposerKey
    public private(set) var lifecycle: GaryxPayloadLifecycleRecord
    public private(set) var currentGeneration: UInt64
    public private(set) var textByGeneration: [UInt64: String]
    public private(set) var attachments: [GaryxAttachmentID: GaryxComposerAttachment]
    public private(set) var operationKeys: Set<GaryxOperationCapabilityKey>
    public private(set) var deliveryReferences: Set<GaryxDeliveryRecordID>
    public private(set) var feedbackReferences: Set<GaryxFeedbackID>
    public private(set) var aliasReferenceCount: Int

    public init(
        id: GaryxComposerPayloadEntryID,
        scope: GaryxGatewayScope,
        destination: GaryxComposerKey,
        lifecycleToken: GaryxPayloadLifecycleToken,
        currentGeneration: UInt64,
        text: String = ""
    ) {
        precondition(lifecycleToken.entryID == id)
        self.id = id
        self.scope = scope
        self.destination = destination
        lifecycle = GaryxPayloadLifecycleRecord(token: lifecycleToken)
        self.currentGeneration = currentGeneration
        textByGeneration = text.isEmpty ? [:] : [currentGeneration: text]
        attachments = [:]
        operationKeys = []
        deliveryReferences = []
        feedbackReferences = []
        aliasReferenceCount = 0
    }

    public var currentText: String { textByGeneration[currentGeneration] ?? "" }

    /// A payload is safe to replace automatically only when no visible text,
    /// attachment, or in-flight attachment producer represents the user's
    /// current intent. Whitespace-only text follows composer send semantics
    /// and is considered blank.
    public var hasMeaningfulCurrentPayload: Bool {
        !currentText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !attachments.isEmpty
            || !operationKeys.isEmpty
    }

    public var isReclaimable: Bool {
        textByGeneration.values.allSatisfy(\.isEmpty)
            && attachments.isEmpty
            && operationKeys.isEmpty
            && deliveryReferences.isEmpty
            && feedbackReferences.isEmpty
            && aliasReferenceCount == 0
    }

    public mutating func promote(to destination: GaryxComposerKey) {
        self.destination = destination
    }

    public mutating func setText(_ text: String, generation: UInt64) {
        if text.isEmpty {
            textByGeneration.removeValue(forKey: generation)
        } else {
            textByGeneration[generation] = text
        }
        currentGeneration = max(currentGeneration, generation)
    }

    public mutating func addAttachment(_ attachment: GaryxComposerAttachment) {
        attachments[attachment.id] = attachment
    }

    public mutating func removeAttachment(_ id: GaryxAttachmentID) {
        attachments.removeValue(forKey: id)
    }

    /// Explicit conflict resolution may replace the currently edited payload,
    /// but only after the caller has durably admitted both candidates. Keeping
    /// this as one Entry mutation prevents a recovered send from partially
    /// overwriting a live follow-up draft.
    @discardableResult
    public mutating func replaceCurrentPayload(
        text: String,
        attachments: [GaryxComposerAttachment],
        generation: UInt64
    ) -> Bool {
        guard lifecycle.phase == .active, generation > currentGeneration else {
            return false
        }
        textByGeneration.removeAll()
        self.attachments.removeAll()
        currentGeneration = generation
        if !text.isEmpty {
            textByGeneration[generation] = text
        }
        for attachment in attachments {
            self.attachments[attachment.id] = GaryxComposerAttachment(
                id: attachment.id,
                stagedAssetID: attachment.stagedAssetID,
                generation: generation,
                byteCount: attachment.byteCount,
                kind: attachment.kind,
                name: attachment.name,
                mediaType: attachment.mediaType,
                uploadedPath: attachment.uploadedPath,
                previewDataURL: attachment.previewDataURL
            )
        }
        return true
    }

    /// Starts the next editable generation after the previous payload has been
    /// handed to the delivery transaction. The Entry identity remains stable;
    /// empty text therefore never deletes the key or loses its per-key slot.
    @discardableResult
    public mutating func beginFreshGeneration(_ generation: UInt64) -> Bool {
        guard lifecycle.phase == .active,
              generation > currentGeneration,
              operationKeys.isEmpty else {
            return false
        }
        textByGeneration.removeAll()
        attachments.removeAll()
        currentGeneration = generation
        return true
    }

    public mutating func addOperation(_ key: GaryxOperationCapabilityKey) {
        operationKeys.insert(key)
    }

    public mutating func removeOperation(_ key: GaryxOperationCapabilityKey) {
        operationKeys.remove(key)
    }

    public mutating func addDeliveryReference(_ id: GaryxDeliveryRecordID) {
        deliveryReferences.insert(id)
    }

    public mutating func removeDeliveryReference(_ id: GaryxDeliveryRecordID) {
        deliveryReferences.remove(id)
    }

    /// Publishes the payload side of `commitSend`: the sealed generation is
    /// removed from composer state, the provisional follow-up becomes the
    /// current generation, and the immutable outbox record keeps the Entry
    /// alive. The caller must persist this value in the same transaction as
    /// the committed reservation ledger and DeliveryRecord.
    @discardableResult
    mutating func settleCommittedSend(
        envelopeGeneration: UInt64,
        followupGeneration: UInt64,
        followupText: String,
        followupAttachmentIDs: [GaryxAttachmentID],
        deliveryID: GaryxDeliveryRecordID
    ) -> Bool {
        guard lifecycle.phase == .active,
              followupGeneration > envelopeGeneration,
              currentGeneration == envelopeGeneration
                || currentGeneration == followupGeneration else {
            return false
        }

        textByGeneration.removeValue(forKey: envelopeGeneration)
        textByGeneration.removeValue(forKey: followupGeneration)
        if !followupText.isEmpty {
            textByGeneration[followupGeneration] = followupText
        }
        let followupIDs = Set(followupAttachmentIDs)
        attachments = attachments.filter { id, attachment in
            attachment.generation != envelopeGeneration
                && (attachment.generation != followupGeneration || followupIDs.contains(id))
        }
        currentGeneration = followupGeneration
        deliveryReferences.insert(deliveryID)
        return true
    }

    public mutating func addFeedbackReference(_ id: GaryxFeedbackID) {
        feedbackReferences.insert(id)
    }

    public mutating func removeFeedbackReference(_ id: GaryxFeedbackID) {
        feedbackReferences.remove(id)
    }

    public mutating func setAliasReferenceCount(_ count: Int) {
        precondition(count >= 0)
        aliasReferenceCount = count
    }

    @discardableResult
    public mutating func beginDiscard(revision: UInt64) -> Bool {
        lifecycle.beginDiscard(discardRevision: revision)
    }

    @discardableResult
    mutating func resetGeneration(
        _ generation: UInt64,
        to allocatedGeneration: UInt64,
        barrierIdle: Bool,
        producerLive: Bool
    ) -> Bool {
        guard lifecycle.phase == .active,
              barrierIdle,
              producerLive,
              generation == currentGeneration,
              allocatedGeneration > generation,
              !operationKeys.contains(where: { $0.generation == generation }) else {
            return false
        }
        textByGeneration.removeValue(forKey: generation)
        attachments = attachments.filter { $0.value.generation != generation }
        operationKeys = Set(operationKeys.filter { $0.generation != generation })
        currentGeneration = allocatedGeneration
        return true
    }

    mutating func recoverSyntheticRevocation(
        envelopeGeneration: UInt64,
        followupGeneration: UInt64,
        mergeGeneration: UInt64,
        mergedText: String
    ) {
        precondition(mergeGeneration > followupGeneration)
        textByGeneration.removeValue(forKey: envelopeGeneration)
        textByGeneration.removeValue(forKey: followupGeneration)
        if mergedText.isEmpty {
            textByGeneration.removeValue(forKey: mergeGeneration)
        } else {
            textByGeneration[mergeGeneration] = mergedText
        }
        attachments = Dictionary(uniqueKeysWithValues: attachments.values.map { attachment in
            guard attachment.generation == envelopeGeneration
                    || attachment.generation == followupGeneration else {
                return (attachment.id, attachment)
            }
            let remapped = GaryxComposerAttachment(
                id: attachment.id,
                stagedAssetID: attachment.stagedAssetID,
                generation: mergeGeneration,
                byteCount: attachment.byteCount,
                kind: attachment.kind,
                name: attachment.name,
                mediaType: attachment.mediaType,
                uploadedPath: attachment.uploadedPath,
                previewDataURL: attachment.previewDataURL
            )
            return (remapped.id, remapped)
        })
        currentGeneration = mergeGeneration
    }

    mutating func remapOperationKey(
        from oldKey: GaryxOperationCapabilityKey,
        to newKey: GaryxOperationCapabilityKey
    ) {
        guard operationKeys.remove(oldKey) != nil else { return }
        operationKeys.insert(newKey)
    }
}

public enum GaryxPayloadMutationKind: String, CaseIterable, Codable, Sendable {
    case presentationResult
    case manifestAdmission
    case pendingReplacement
    case replacementSwap
    case admitFreshOperation
    case operationTransition
    case operationCompletion
    case inputEdit
    case inputClose
    case beginSend
    case generationReset
    case inputDrain
    case producerDrained
    case dualTerminalTransaction
}

public enum GaryxPayloadMutationAdmission: Equatable, Sendable {
    case admitted
    case rejectedLifecycle
}

public enum GaryxPayloadMutationGate {
    public static func admit(
        _ kind: GaryxPayloadMutationKind,
        capture: GaryxPayloadLifecycleCapture,
        lifecycle: GaryxPayloadLifecycleSnapshot
    ) -> GaryxPayloadMutationAdmission {
        capture.isAdmitted(by: lifecycle) ? .admitted : .rejectedLifecycle
    }
}

public struct GaryxComposerPayloadStore: Equatable, Codable, Sendable {
    public fileprivate(set) var entriesByScope: [GaryxGatewayScope: [GaryxComposerPayloadEntryID: GaryxComposerPayloadEntry]]

    public init() {
        entriesByScope = [:]
    }

    public func entry(
        _ id: GaryxComposerPayloadEntryID,
        scope: GaryxGatewayScope
    ) -> GaryxComposerPayloadEntry? {
        entriesByScope[scope]?[id]
    }

    @discardableResult
    public mutating func insert(_ entry: GaryxComposerPayloadEntry) -> Bool {
        guard entriesByScope[entry.scope]?[entry.id] == nil else { return false }
        entriesByScope[entry.scope, default: [:]][entry.id] = entry
        return true
    }

    public mutating func update(_ entry: GaryxComposerPayloadEntry) {
        precondition(entriesByScope[entry.scope]?[entry.id] != nil)
        entriesByScope[entry.scope]?[entry.id] = entry
    }

    @discardableResult
    public mutating func remove(
        _ entryID: GaryxComposerPayloadEntryID,
        scope: GaryxGatewayScope
    ) -> GaryxComposerPayloadEntry? {
        let removed = entriesByScope[scope]?.removeValue(forKey: entryID)
        if entriesByScope[scope]?.isEmpty == true { entriesByScope.removeValue(forKey: scope) }
        return removed
    }

    @discardableResult
    public mutating func promote(
        entryID: GaryxComposerPayloadEntryID,
        scope: GaryxGatewayScope,
        to target: GaryxComposerKey
    ) -> Bool {
        guard var entry = entriesByScope[scope]?[entryID],
              entry.lifecycle.phase == .active,
              !hasPromotionCollision(entryID: entryID, scope: scope, target: target) else {
            return false
        }
        let stableToken = entry.lifecycle.token
        entry.promote(to: target)
        precondition(entry.lifecycle.token == stableToken, "promotion must preserve EntryID/token")
        entriesByScope[scope]?[entryID] = entry
        return true
    }

    fileprivate func hasPromotionCollision(
        entryID: GaryxComposerPayloadEntryID,
        scope: GaryxGatewayScope,
        target: GaryxComposerKey
    ) -> Bool {
        entriesByScope[scope]?.values.contains(where: {
            $0.id != entryID && $0.destination == target
        }) == true
    }

    fileprivate mutating func promoteAfterConflictAdmission(
        entryID: GaryxComposerPayloadEntryID,
        scope: GaryxGatewayScope,
        to target: GaryxComposerKey
    ) -> Bool {
        guard var entry = entriesByScope[scope]?[entryID], entry.lifecycle.phase == .active else {
            return false
        }
        entry.promote(to: target)
        entriesByScope[scope]?[entryID] = entry
        return true
    }

    @discardableResult
    public mutating func removeIfReclaimable(
        _ entryID: GaryxComposerPayloadEntryID,
        scope: GaryxGatewayScope
    ) -> Bool {
        guard entriesByScope[scope]?[entryID]?.isReclaimable == true else { return false }
        entriesByScope[scope]?.removeValue(forKey: entryID)
        if entriesByScope[scope]?.isEmpty == true { entriesByScope.removeValue(forKey: scope) }
        return true
    }
}

// MARK: - Scope-bound operation capability

public struct GaryxOperationCapabilityKey: Hashable, Codable, Sendable {
    public let scope: GaryxGatewayScope
    public let entryID: GaryxComposerPayloadEntryID
    public let generation: UInt64
    public let reservationID: GaryxSendReservationID?
    public let branch: GaryxOperationBranch
    public let operationID: GaryxOperationID

    public init(
        scope: GaryxGatewayScope,
        entryID: GaryxComposerPayloadEntryID,
        generation: UInt64,
        reservationID: GaryxSendReservationID?,
        branch: GaryxOperationBranch,
        operationID: GaryxOperationID
    ) {
        self.scope = scope
        self.entryID = entryID
        self.generation = generation
        self.reservationID = reservationID
        self.branch = branch
        self.operationID = operationID
    }

    func remapped(toGeneration generation: UInt64) -> Self {
        Self(
            scope: scope,
            entryID: entryID,
            generation: generation,
            reservationID: reservationID,
            branch: branch,
            operationID: operationID
        )
    }
}

public struct GaryxScopeBoundOperationContext: Equatable, Codable, Sendable {
    public let key: GaryxOperationCapabilityKey
    public let clientIdentity: String
    public let configurationFingerprint: String
    public let payloadLifecycle: GaryxPayloadLifecycleCapture

    public init(
        key: GaryxOperationCapabilityKey,
        clientIdentity: String,
        configurationFingerprint: String,
        payloadLifecycle: GaryxPayloadLifecycleCapture
    ) {
        self.key = key
        self.clientIdentity = clientIdentity
        self.configurationFingerprint = configurationFingerprint
        self.payloadLifecycle = payloadLifecycle
    }

    public func replacingOperationID(
        _ operationID: GaryxOperationID
    ) -> GaryxScopeBoundOperationContext {
        GaryxScopeBoundOperationContext(
            key: GaryxOperationCapabilityKey(
                scope: key.scope,
                entryID: key.entryID,
                generation: key.generation,
                reservationID: key.reservationID,
                branch: key.branch,
                operationID: operationID
            ),
            clientIdentity: clientIdentity,
            configurationFingerprint: configurationFingerprint,
            payloadLifecycle: payloadLifecycle
        )
    }

    func remapped(toGeneration generation: UInt64) -> Self {
        Self(
            key: key.remapped(toGeneration: generation),
            clientIdentity: clientIdentity,
            configurationFingerprint: configurationFingerprint,
            payloadLifecycle: payloadLifecycle
        )
    }
}

public enum GaryxOperationCapabilityState: String, CaseIterable, Codable, Sendable {
    case requested
    case preparing
    case uploading
    case completed
    case failedRetryable
    case failedTerminal
    case cancelled
    case superseded

    public var blocksSend: Bool {
        switch self {
        case .requested, .preparing, .uploading, .failedRetryable:
            true
        case .completed, .failedTerminal, .cancelled, .superseded:
            false
        }
    }
}

public enum GaryxOperationTransitionDisposition: Equatable, Sendable {
    case applied
    case rejectedKey
    case rejectedLifecycle
    case rejectedScope
    case rejectedState
    case archivedIdentityInvalid
}

public struct GaryxOperationCapability: Equatable, Codable, Sendable {
    public let context: GaryxScopeBoundOperationContext
    public private(set) var state: GaryxOperationCapabilityState
    public private(set) var stagedAssetID: GaryxStagedAssetID?
    public private(set) var reservedBytes: Int
    public private(set) var uploadAttempted: Bool
    public private(set) var supersededBy: GaryxOperationID?
    public private(set) var identityValid: Bool

    public init(
        context: GaryxScopeBoundOperationContext,
        state: GaryxOperationCapabilityState = .requested,
        stagedAssetID: GaryxStagedAssetID? = nil,
        reservedBytes: Int = 0,
        uploadAttempted: Bool = false
    ) {
        precondition(reservedBytes >= 0)
        self.context = context
        self.state = state
        self.stagedAssetID = stagedAssetID
        self.reservedBytes = reservedBytes
        self.uploadAttempted = uploadAttempted
        supersededBy = nil
        identityValid = true
    }

    @discardableResult
    public mutating func markUploadAttempted(
        expectedKey: GaryxOperationCapabilityKey,
        authoritativeEntry: GaryxComposerPayloadEntry,
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) -> GaryxOperationTransitionDisposition {
        guard expectedKey == context.key else { return .rejectedKey }
        guard identityValid,
              authoritativeEntry.id == context.key.entryID,
              authoritativeEntry.scope == context.key.scope,
              authoritativeEntry.lifecycle.token == context.payloadLifecycle.token,
              authoritativeEntry.operationKeys.contains(context.key) else {
            settleIdentityDiscard()
            return .archivedIdentityInvalid
        }
        guard context.payloadLifecycle.isAdmitted(by: lifecycle) else { return .rejectedLifecycle }
        guard scopes.admitDomainEvent(from: context.key.scope) != .rejectedRevoked else {
            return .rejectedScope
        }
        guard state == .uploading, !uploadAttempted else { return .rejectedState }
        uploadAttempted = true
        return .applied
    }

    fileprivate mutating func adoptReplacementAsset(
        _ stagedAssetID: GaryxStagedAssetID,
        reservedBytes: Int
    ) {
        guard state == .requested else { return }
        precondition(reservedBytes >= 0)
        self.stagedAssetID = stagedAssetID
        self.reservedBytes = reservedBytes
    }

    public mutating func invalidateIdentity() {
        identityValid = false
    }

    @discardableResult
    public mutating func transition(
        expectedKey: GaryxOperationCapabilityKey,
        to next: GaryxOperationCapabilityState,
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry,
        supersededBy: GaryxOperationID? = nil
    ) -> GaryxOperationTransitionDisposition {
        guard expectedKey == context.key else { return .rejectedKey }
        guard context.payloadLifecycle.isAdmitted(by: lifecycle) else { return .rejectedLifecycle }
        guard scopes.admitDomainEvent(from: context.key.scope) != .rejectedRevoked else {
            return .rejectedScope
        }
        guard identityValid else { return .archivedIdentityInvalid }
        guard Self.allows(from: state, to: next) else { return .rejectedState }
        state = next
        if next == .superseded {
            self.supersededBy = supersededBy
            // The swap transaction has already transferred physical-file and
            // quota ownership to O2. O1 remains as lineage only and therefore
            // must not retain the condemned-owner shape.
            stagedAssetID = nil
            reservedBytes = 0
        }
        return .applied
    }

    /// Completion's operation x scope x identity triple CAS.
    @discardableResult
    public mutating func complete(
        expectedKey: GaryxOperationCapabilityKey,
        authoritativeEntry: GaryxComposerPayloadEntry,
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) -> GaryxOperationTransitionDisposition {
        guard expectedKey == context.key else { return .rejectedKey }
        guard identityValid,
              authoritativeEntry.id == context.key.entryID,
              authoritativeEntry.scope == context.key.scope,
              authoritativeEntry.lifecycle.token == context.payloadLifecycle.token,
              authoritativeEntry.operationKeys.contains(context.key) else {
            settleIdentityDiscard()
            return .archivedIdentityInvalid
        }
        guard context.payloadLifecycle.isAdmitted(by: lifecycle) else { return .rejectedLifecycle }
        guard scopes.admitDomainEvent(from: context.key.scope) != .rejectedRevoked else {
            return .rejectedScope
        }
        guard state == .uploading else { return .rejectedState }
        state = .completed
        return .applied
    }

    public mutating func settleIdentityDiscard() {
        identityValid = false
        switch state {
        case .requested, .preparing, .uploading, .failedRetryable:
            state = .cancelled
        case .completed, .failedTerminal, .cancelled, .superseded:
            break
        }
        // Identity settlement owns resource cleanup even when the operation
        // reached a terminal state first. Superseded records only describe
        // lineage; their successor is settled independently.
        stagedAssetID = nil
        reservedBytes = 0
    }

    func remapped(toGeneration generation: UInt64) -> Self {
        var result = Self(
            context: context.remapped(toGeneration: generation),
            state: state,
            stagedAssetID: stagedAssetID,
            reservedBytes: reservedBytes,
            uploadAttempted: uploadAttempted
        )
        result.supersededBy = supersededBy
        result.identityValid = identityValid
        return result
    }

    private static func allows(
        from: GaryxOperationCapabilityState,
        to: GaryxOperationCapabilityState
    ) -> Bool {
        switch (from, to) {
        case (.requested, .preparing), (.requested, .cancelled),
             (.preparing, .uploading), (.preparing, .failedTerminal),
             (.preparing, .cancelled),
             (.uploading, .completed), (.uploading, .failedRetryable),
             (.uploading, .failedTerminal), (.uploading, .cancelled),
             (.failedRetryable, .superseded), (.failedRetryable, .cancelled):
            true
        default:
            false
        }
    }
}

public struct GaryxOperationManifest: Equatable, Codable, Sendable {
    public let key: GaryxOperationCapabilityKey
    public let stagedPath: String
    public let state: GaryxOperationCapabilityState
    public let uploadAttempted: Bool

    public init(
        key: GaryxOperationCapabilityKey,
        stagedPath: String,
        state: GaryxOperationCapabilityState,
        uploadAttempted: Bool
    ) {
        self.key = key
        self.stagedPath = stagedPath
        self.state = state
        self.uploadAttempted = uploadAttempted
    }

    func remapped(toGeneration generation: UInt64) -> Self {
        Self(
            key: key.remapped(toGeneration: generation),
            stagedPath: stagedPath,
            state: state,
            uploadAttempted: uploadAttempted
        )
    }
}

public enum GaryxOperationRecoveryDecision: Equatable, Sendable {
    case cancelAndCleanStaging(erasePayload: Bool)
    case retryBeforeTransport
    case suspendInOriginPartition
    case failedRetryableWithFeedback
    case archiveAttemptedUploadEvidence
    case archiveCompletedPayloadEvidence
    case placeCompletedAndCleanStaging
    case preserveFailedRetryable
    case persistFailedTerminalFeedback
    case cleanAndArchiveWithoutUI
    case cleanOperationChild
    case ownershipTransferred
    case settleSuccessorForRevocation
}

public enum GaryxOperationRecoveryPlanner {
    public static func decide(
        state: GaryxOperationCapabilityState,
        uploadAttempted: Bool,
        scope: GaryxGatewayScopeLifecycle
    ) -> GaryxOperationRecoveryDecision {
        switch (state, scope) {
        case (.requested, .active), (.preparing, .active),
             (.requested, .suspended), (.preparing, .suspended):
            return .cancelAndCleanStaging(erasePayload: false)
        case (.requested, .revoked), (.preparing, .revoked):
            return .cancelAndCleanStaging(erasePayload: true)
        case (.uploading, .active):
            return uploadAttempted ? .failedRetryableWithFeedback : .retryBeforeTransport
        case (.uploading, .suspended):
            return uploadAttempted ? .failedRetryableWithFeedback : .suspendInOriginPartition
        case (.uploading, .revoked):
            return uploadAttempted
                ? .archiveAttemptedUploadEvidence
                : .cancelAndCleanStaging(erasePayload: true)
        case (.completed, .active), (.completed, .suspended):
            return .placeCompletedAndCleanStaging
        case (.completed, .revoked):
            return .archiveCompletedPayloadEvidence
        case (.failedRetryable, .active), (.failedRetryable, .suspended):
            return .preserveFailedRetryable
        case (.failedRetryable, .revoked):
            return .cleanOperationChild
        case (.failedTerminal, .active), (.failedTerminal, .suspended):
            return .persistFailedTerminalFeedback
        case (.failedTerminal, .revoked):
            return .cleanAndArchiveWithoutUI
        case (.cancelled, _):
            return .cleanOperationChild
        case (.superseded, .active), (.superseded, .suspended):
            return .ownershipTransferred
        case (.superseded, .revoked):
            return .settleSuccessorForRevocation
        }
    }
}

public enum GaryxComposerSendReadiness: Equatable, Sendable {
    case ready
    case payloadPreparing
}

public enum GaryxComposerSendReadinessPolicy {
    public static func evaluate(_ operations: some Sequence<GaryxOperationCapability>) -> GaryxComposerSendReadiness {
        operations.contains(where: { $0.state.blocksSend }) ? .payloadPreparing : .ready
    }
}

// MARK: - Replacement journal

public enum GaryxReplacementPhase: String, Codable, Sendable {
    case pendingReplacement
    case committed
    case aborted
    case settled
}

public struct GaryxReplacementRecord: Equatable, Codable, Sendable {
    public let id: GaryxReplacementID
    public let scope: GaryxGatewayScope
    public let entryID: GaryxComposerPayloadEntryID
    public let oldKey: GaryxOperationCapabilityKey
    public private(set) var newKey: GaryxOperationCapabilityKey?
    public let reservationID: GaryxSendReservationID?
    public let branch: GaryxOperationBranch
    public let stagedAssetID: GaryxStagedAssetID
    public let reservedBytes: Int
    public private(set) var phase: GaryxReplacementPhase

    public init(
        id: GaryxReplacementID,
        scope: GaryxGatewayScope,
        entryID: GaryxComposerPayloadEntryID,
        oldKey: GaryxOperationCapabilityKey,
        newKey: GaryxOperationCapabilityKey? = nil,
        reservationID: GaryxSendReservationID?,
        branch: GaryxOperationBranch,
        stagedAssetID: GaryxStagedAssetID,
        reservedBytes: Int,
        phase: GaryxReplacementPhase = .pendingReplacement
    ) {
        precondition(reservedBytes >= 0)
        self.id = id
        self.scope = scope
        self.entryID = entryID
        self.oldKey = oldKey
        self.newKey = newKey
        self.reservationID = reservationID
        self.branch = branch
        self.stagedAssetID = stagedAssetID
        self.reservedBytes = reservedBytes
        self.phase = phase
    }

    public mutating func commit(newKey: GaryxOperationCapabilityKey) {
        guard phase == .pendingReplacement,
              hasValidOldKey,
              newKey.scope == scope,
              newKey.entryID == entryID,
              newKey.generation == oldKey.generation,
              newKey.reservationID == reservationID,
              newKey.branch == branch,
              newKey.operationID != oldKey.operationID else {
            return
        }
        self.newKey = newKey
        phase = .committed
    }

    public mutating func abort() {
        guard phase == .pendingReplacement else { return }
        phase = .aborted
    }

    public mutating func settle() {
        guard phase == .committed || phase == .aborted else { return }
        phase = .settled
    }

    public var hasValidOldKey: Bool {
        oldKey.scope == scope
            && oldKey.entryID == entryID
            && oldKey.reservationID == reservationID
            && oldKey.branch == branch
    }

    public var hasValidCommittedKey: Bool {
        guard let newKey else { return false }
        return hasValidOldKey
            && newKey.scope == scope
            && newKey.entryID == entryID
            && newKey.generation == oldKey.generation
            && newKey.reservationID == reservationID
            && newKey.branch == branch
            && newKey.operationID != oldKey.operationID
    }

    func remapped(
        from oldOperationKey: GaryxOperationCapabilityKey,
        to newOperationKey: GaryxOperationCapabilityKey
    ) -> Self {
        Self(
            id: id,
            scope: scope,
            entryID: entryID,
            oldKey: oldKey == oldOperationKey ? newOperationKey : oldKey,
            newKey: newKey == oldOperationKey ? newOperationKey : newKey,
            reservationID: reservationID,
            branch: branch,
            stagedAssetID: stagedAssetID,
            reservedBytes: reservedBytes,
            phase: phase
        )
    }
}

public enum GaryxReplacementRecoveryDecision: Equatable, Sendable {
    case abortReleaseQuotaAndDeleteProvisional
    case restoreSuccessor(GaryxOperationCapabilityKey)
    case garbageCollect
}

public enum GaryxReplacementReclamationDecision: Equatable, Sendable {
    case reclaim
    case retainActiveManifest
    case awaitSuccessorOwnerTransaction
}

public enum GaryxReplacementPlanner {
    public static func recover(_ record: GaryxReplacementRecord) -> GaryxReplacementRecoveryDecision {
        switch record.phase {
        case .pendingReplacement, .aborted:
            .abortReleaseQuotaAndDeleteProvisional
        case .committed:
            record.hasValidCommittedKey
                ? .restoreSuccessor(record.newKey!)
                : .abortReleaseQuotaAndDeleteProvisional
        case .settled:
            .garbageCollect
        }
    }

    public static func reclaim(
        successorState: GaryxOperationCapabilityState,
        scope: GaryxGatewayScopeLifecycle
    ) -> GaryxReplacementReclamationDecision {
        if scope == .revoked { return .reclaim }
        switch successorState {
        case .completed, .failedTerminal, .cancelled:
            return .reclaim
        case .superseded:
            return .awaitSuccessorOwnerTransaction
        case .failedRetryable:
            return .retainActiveManifest
        case .requested, .preparing, .uploading:
            return .retainActiveManifest
        }
    }
}

public enum GaryxReplacementSwapDisposition: Equatable, Sendable {
    case committed
    case rejectedLifecycle
    case rejectedScope
    case rejectedOldOperation
    case rejectedNewOperation
}

public enum GaryxReplacementSwapReducer {
    /// Atomic value reducer. Inputs are copied and assigned only after all
    /// checks and state transitions succeed, so failure preserves O1 exactly.
    public static func commit(
        old: inout GaryxOperationCapability,
        successor: inout GaryxOperationCapability,
        record: inout GaryxReplacementRecord,
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) -> GaryxReplacementSwapDisposition {
        guard old.context.payloadLifecycle.isAdmitted(by: lifecycle),
              successor.context.payloadLifecycle.isAdmitted(by: lifecycle) else {
            return .rejectedLifecycle
        }
        guard scopes.admitDomainEvent(from: old.context.key.scope) != .rejectedRevoked,
              scopes.admitDomainEvent(from: successor.context.key.scope) != .rejectedRevoked else {
            return .rejectedScope
        }
        guard old.state == .failedRetryable,
              old.context.key == record.oldKey,
              record.hasValidOldKey else {
            return .rejectedOldOperation
        }
        guard successor.state == .requested,
              successor.context.key.scope == record.scope,
              successor.context.key.entryID == record.entryID,
              successor.context.key.generation == record.oldKey.generation,
              successor.context.key.reservationID == record.reservationID,
              successor.context.key.branch == record.branch,
              successor.context.key.operationID != record.oldKey.operationID,
              record.phase == .pendingReplacement else {
            return .rejectedNewOperation
        }

        var nextOld = old
        var nextSuccessor = successor
        var nextRecord = record
        nextSuccessor.adoptReplacementAsset(
            record.stagedAssetID,
            reservedBytes: record.reservedBytes
        )
        guard nextOld.transition(
            expectedKey: nextOld.context.key,
            to: .superseded,
            lifecycle: lifecycle,
            scopes: scopes,
            supersededBy: nextSuccessor.context.key.operationID
        ) == .applied else {
            return .rejectedOldOperation
        }
        guard nextSuccessor.transition(
            expectedKey: nextSuccessor.context.key,
            to: .preparing,
            lifecycle: lifecycle,
            scopes: scopes
        ) == .applied else {
            return .rejectedNewOperation
        }
        nextRecord.commit(newKey: nextSuccessor.context.key)
        old = nextOld
        successor = nextSuccessor
        record = nextRecord
        return .committed
    }
}

// MARK: - Durable feedback

public enum GaryxOperationFeedbackKind: String, Codable, Sendable {
    case uploadRetryable
    case uploadTerminal
    case quotaExceeded
    case deliveryBackpressure
    case deliveryAttachmentRecoveryIncomplete
}

public enum GaryxOperationFeedbackPhase: String, Codable, Sendable {
    case pending
    case presented
    case acknowledged
    case archived
}

public struct GaryxOperationFeedback: Equatable, Codable, Sendable {
    public let id: GaryxFeedbackID
    public let scope: GaryxGatewayScope
    public let entryID: GaryxComposerPayloadEntryID
    public let operationID: GaryxOperationID?
    public let lineageID: GaryxAttachmentLineageID?
    public let kind: GaryxOperationFeedbackKind
    public private(set) var phase: GaryxOperationFeedbackPhase

    public init(
        id: GaryxFeedbackID,
        scope: GaryxGatewayScope,
        entryID: GaryxComposerPayloadEntryID,
        operationID: GaryxOperationID?,
        lineageID: GaryxAttachmentLineageID? = nil,
        kind: GaryxOperationFeedbackKind,
        phase: GaryxOperationFeedbackPhase = .pending
    ) {
        self.id = id
        self.scope = scope
        self.entryID = entryID
        self.operationID = operationID
        self.lineageID = lineageID
        self.kind = kind
        self.phase = phase
    }

    public var isTerminal: Bool { phase == .acknowledged || phase == .archived }

    @discardableResult
    public mutating func present(
        hostEntryID: GaryxComposerPayloadEntryID,
        hasInteractionOwner: Bool
    ) -> Bool {
        guard phase == .pending, hostEntryID == entryID, hasInteractionOwner else { return false }
        phase = .presented
        return true
    }

    public mutating func acknowledge() {
        guard phase == .pending || phase == .presented else { return }
        phase = .acknowledged
    }

    public mutating func archive() {
        guard !isTerminal else { return }
        phase = .archived
    }
}

public enum GaryxAttachmentLineagePhase: String, Codable, Sendable {
    case retainedForFeedback
    case released
}

/// Stable attachment-slot metadata retained after a failed-terminal operation.
/// It carries no staged path or payload bytes. A fresh operation may reuse the
/// slot only while both lifecycle and feedback CAS still match.
public struct GaryxAttachmentLineageTombstone: Equatable, Codable, Sendable {
    public let id: GaryxAttachmentLineageID
    public let scope: GaryxGatewayScope
    public let entryID: GaryxComposerPayloadEntryID
    public let attachmentSlotID: GaryxAttachmentID
    public let failedOperationID: GaryxOperationID
    public let feedbackID: GaryxFeedbackID
    public let payloadLifecycle: GaryxPayloadLifecycleCapture
    public private(set) var phase: GaryxAttachmentLineagePhase

    public init(
        id: GaryxAttachmentLineageID,
        scope: GaryxGatewayScope,
        entryID: GaryxComposerPayloadEntryID,
        attachmentSlotID: GaryxAttachmentID,
        failedOperationID: GaryxOperationID,
        feedbackID: GaryxFeedbackID,
        payloadLifecycle: GaryxPayloadLifecycleCapture
    ) {
        self.id = id
        self.scope = scope
        self.entryID = entryID
        self.attachmentSlotID = attachmentSlotID
        self.failedOperationID = failedOperationID
        self.feedbackID = feedbackID
        self.payloadLifecycle = payloadLifecycle
        phase = .retainedForFeedback
    }

    public func admitsFreshOperation(
        _ operation: GaryxOperationCapability,
        feedback: GaryxOperationFeedback,
        lifecycle: GaryxPayloadLifecycleSnapshot
    ) -> Bool {
        phase == .retainedForFeedback
            && payloadLifecycle.isAdmitted(by: lifecycle)
            && operation.context.key.scope == scope
            && operation.context.key.entryID == entryID
            && operation.context.payloadLifecycle == payloadLifecycle
            && operation.state == .requested
            && feedback.id == feedbackID
            && feedback.lineageID == id
            && feedback.scope == scope
            && feedback.entryID == entryID
            && !feedback.isTerminal
    }

    @discardableResult
    public mutating func release(after feedback: GaryxOperationFeedback) -> Bool {
        guard phase == .retainedForFeedback,
              feedback.id == feedbackID,
              feedback.lineageID == id,
              feedback.isTerminal else {
            return false
        }
        phase = .released
        return true
    }
}

public enum GaryxReplacementFeedbackSwapDisposition: Equatable, Sendable {
    case committed
    case rejectedFeedback
    case rejectedSwap(GaryxReplacementSwapDisposition)
}

/// Binds the retryable feedback acknowledgement to the successful replacement
/// swap. No input is published unless O1, O2, the journal, and the chip all
/// reach their next states together.
public enum GaryxReplacementFeedbackSwapReducer {
    public static func commit(
        old: inout GaryxOperationCapability,
        successor: inout GaryxOperationCapability,
        record: inout GaryxReplacementRecord,
        feedback: inout GaryxOperationFeedback,
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) -> GaryxReplacementFeedbackSwapDisposition {
        guard !feedback.isTerminal,
              feedback.scope == record.scope,
              feedback.entryID == record.entryID,
              feedback.operationID == old.context.key.operationID,
              feedback.kind == .uploadRetryable else {
            return .rejectedFeedback
        }

        var nextOld = old
        var nextSuccessor = successor
        var nextRecord = record
        var nextFeedback = feedback
        let swap = GaryxReplacementSwapReducer.commit(
            old: &nextOld,
            successor: &nextSuccessor,
            record: &nextRecord,
            lifecycle: lifecycle,
            scopes: scopes
        )
        guard swap == .committed else { return .rejectedSwap(swap) }
        nextFeedback.acknowledge()
        guard nextFeedback.phase == .acknowledged else { return .rejectedFeedback }
        old = nextOld
        successor = nextSuccessor
        record = nextRecord
        feedback = nextFeedback
        return .committed
    }
}

public enum GaryxOperationRemovalFeedbackDisposition: Equatable, Sendable {
    case committed
    case rejectedLifecycle
    case rejectedScope
    case rejectedOperation
    case rejectedFeedback
    case rejectedLineage
}

/// Explicit remove is one value transaction: capability cancellation and
/// resource cleanup cannot publish before the durable feedback acknowledgement
/// (and failed-terminal lineage release), or vice versa.
public enum GaryxOperationRemovalFeedbackReducer {
    public static func commit(
        operation: inout GaryxOperationCapability,
        feedback: inout GaryxOperationFeedback,
        lineage: inout GaryxAttachmentLineageTombstone?,
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) -> GaryxOperationRemovalFeedbackDisposition {
        guard operation.context.payloadLifecycle.isAdmitted(by: lifecycle) else {
            return .rejectedLifecycle
        }
        guard scopes.admitDomainEvent(from: operation.context.key.scope) != .rejectedRevoked else {
            return .rejectedScope
        }
        let expectedFeedbackKind: GaryxOperationFeedbackKind
        switch operation.state {
        case .failedRetryable:
            expectedFeedbackKind = .uploadRetryable
        case .failedTerminal:
            expectedFeedbackKind = .uploadTerminal
        case .requested, .preparing, .uploading, .completed, .cancelled, .superseded:
            return .rejectedOperation
        }
        guard !feedback.isTerminal,
              feedback.scope == operation.context.key.scope,
              feedback.entryID == operation.context.key.entryID,
              feedback.operationID == operation.context.key.operationID,
              feedback.kind == expectedFeedbackKind else {
            return .rejectedFeedback
        }
        if let lineageID = feedback.lineageID {
            guard let currentLineage = lineage,
                  currentLineage.id == lineageID,
                  currentLineage.scope == feedback.scope,
                  currentLineage.entryID == feedback.entryID,
                  currentLineage.failedOperationID == operation.context.key.operationID else {
                return .rejectedLineage
            }
        } else if lineage != nil {
            return .rejectedLineage
        }

        var nextOperation = operation
        var nextFeedback = feedback
        var nextLineage = lineage
        nextOperation.settleIdentityDiscard()
        nextFeedback.acknowledge()
        guard nextFeedback.phase == .acknowledged else { return .rejectedFeedback }
        if feedback.lineageID != nil {
            guard nextLineage?.release(after: nextFeedback) == true else {
                return .rejectedLineage
            }
        }
        operation = nextOperation
        feedback = nextFeedback
        lineage = nextLineage
        return .committed
    }
}

public enum GaryxFailedTerminalReattachDisposition: Equatable, Sendable {
    case committed
    case rejectedLineage
    case rejectedOperation(GaryxOperationTransitionDisposition)
}

/// Atomic fresh-operation admission for a failed-terminal attachment slot.
/// Feedback acknowledgement and lineage release cannot get ahead of O2.
public enum GaryxFailedTerminalReattachReducer {
    public static func commit(
        freshOperation: inout GaryxOperationCapability,
        feedback: inout GaryxOperationFeedback,
        lineage: inout GaryxAttachmentLineageTombstone,
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) -> GaryxFailedTerminalReattachDisposition {
        guard lineage.admitsFreshOperation(
            freshOperation,
            feedback: feedback,
            lifecycle: lifecycle
        ), scopes.admitDomainEvent(from: lineage.scope) != .rejectedRevoked else {
            return .rejectedLineage
        }

        var nextOperation = freshOperation
        var nextFeedback = feedback
        var nextLineage = lineage
        let transition = nextOperation.transition(
            expectedKey: nextOperation.context.key,
            to: .preparing,
            lifecycle: lifecycle,
            scopes: scopes
        )
        guard transition == .applied else { return .rejectedOperation(transition) }
        nextFeedback.acknowledge()
        guard nextFeedback.phase == .acknowledged,
              nextLineage.release(after: nextFeedback) else {
            return .rejectedLineage
        }
        freshOperation = nextOperation
        feedback = nextFeedback
        lineage = nextLineage
        return .committed
    }
}

// MARK: - Durable conflict candidates

public struct GaryxPayloadConflictCandidate: Equatable, Codable, Sendable {
    public let entryID: GaryxComposerPayloadEntryID
    public let label: String

    public init(entryID: GaryxComposerPayloadEntryID, label: String) {
        self.entryID = entryID
        self.label = label
    }
}

public struct GaryxPayloadConflictSet: Equatable, Codable, Sendable {
    public let id: GaryxPayloadConflictSetID
    public let scope: GaryxGatewayScope
    public private(set) var candidates: [GaryxPayloadConflictCandidate]

    public init(
        id: GaryxPayloadConflictSetID,
        scope: GaryxGatewayScope,
        candidates: [GaryxPayloadConflictCandidate] = []
    ) {
        self.id = id
        self.scope = scope
        self.candidates = candidates
    }

    @discardableResult
    public mutating func admitCandidate(
        _ candidate: GaryxPayloadConflictCandidate,
        membershipDurabilityAvailable: Bool
    ) -> Bool {
        guard membershipDurabilityAvailable else { return false }
        guard !candidates.contains(where: { $0.entryID == candidate.entryID }) else { return true }
        candidates.append(candidate)
        return true
    }

}

public enum GaryxPayloadPromotionDisposition: Equatable, Sendable {
    case promoted
    case conflictAdmitted(
        conflictSetID: GaryxPayloadConflictSetID,
        candidates: [GaryxComposerPayloadEntryID]
    )
    case rejectedMissingOrInactiveSource
    case rejectedConflictDurability
    case rejectedConflictScope
}

/// Atomic promotion reducer. The plain store API rejects collisions; callers
/// use this reducer to durably admit every colliding EntryID before the source
/// destination is changed. Assignments publish only after the whole value
/// transaction succeeds, so A4d can map the same boundary to one DB commit.
public enum GaryxPayloadPromotionReducer {
    public static func promote(
        entryID: GaryxComposerPayloadEntryID,
        scope: GaryxGatewayScope,
        to target: GaryxComposerKey,
        conflictSetID: GaryxPayloadConflictSetID,
        membershipDurabilityAvailable: Bool,
        store: inout GaryxComposerPayloadStore,
        conflictSets: inout [GaryxPayloadConflictSetID: GaryxPayloadConflictSet]
    ) -> GaryxPayloadPromotionDisposition {
        guard let source = store.entry(entryID, scope: scope),
              source.lifecycle.phase == .active else {
            return .rejectedMissingOrInactiveSource
        }

        let collisions = store.entriesByScope[scope]?.values
            .filter { $0.id != entryID && $0.destination == target }
            .map(\.id)
            .sorted { $0.rawValue < $1.rawValue } ?? []
        guard !collisions.isEmpty else {
            guard store.promote(entryID: entryID, scope: scope, to: target) else {
                return .rejectedMissingOrInactiveSource
            }
            return .promoted
        }
        guard membershipDurabilityAvailable else { return .rejectedConflictDurability }

        var nextStore = store
        var nextConflictSets = conflictSets
        var conflict = nextConflictSets[conflictSetID]
            ?? GaryxPayloadConflictSet(id: conflictSetID, scope: scope)
        guard conflict.scope == scope else { return .rejectedConflictScope }

        let candidateIDs = ([entryID] + collisions).sorted { $0.rawValue < $1.rawValue }
        for candidateID in candidateIDs {
            guard conflict.admitCandidate(
                GaryxPayloadConflictCandidate(
                    entryID: candidateID,
                    label: "promotion-conflict-\(candidateID.rawValue)"
                ),
                membershipDurabilityAvailable: true
            ) else {
                return .rejectedConflictDurability
            }
        }
        guard nextStore.promoteAfterConflictAdmission(
            entryID: entryID,
            scope: scope,
            to: target
        ) else {
            return .rejectedMissingOrInactiveSource
        }
        nextConflictSets[conflictSetID] = conflict
        store = nextStore
        conflictSets = nextConflictSets
        return .conflictAdmitted(conflictSetID: conflictSetID, candidates: candidateIDs)
    }
}

// MARK: - Five identity events

public enum GaryxPayloadIdentityEvent: Equatable, Sendable {
    case aliasSourceRetired(draftID: String)
    case routeOccurrenceSuperseded(GaryxRouteInstanceID)
    case destinationDiscarded(GaryxComposerKey, revision: UInt64)
    case payloadGenerationReset(
        entryID: GaryxComposerPayloadEntryID,
        generation: UInt64,
        allocatedGeneration: UInt64,
        barrierIdle: Bool,
        producerLive: Bool
    )
    case payloadEntryDiscarded(GaryxComposerPayloadEntryID, revision: UInt64)
}

public enum GaryxPayloadIdentityEventDisposition: Equatable, Sendable {
    case aliasOnly
    case occurrenceOnly
    case beganDiscard([GaryxComposerPayloadEntryID])
    case requiresDurableGenerationReset([GaryxOperationCapabilityKey])
    case rejected
}

public enum GaryxPayloadIdentityReducer {
    public static func apply(
        _ event: GaryxPayloadIdentityEvent,
        scope: GaryxGatewayScope,
        store: inout GaryxComposerPayloadStore
    ) -> GaryxPayloadIdentityEventDisposition {
        switch event {
        case .aliasSourceRetired:
            return .aliasOnly
        case .routeOccurrenceSuperseded:
            return .occurrenceOnly
        case .destinationDiscarded(let destination, let revision):
            var discarded: [GaryxComposerPayloadEntryID] = []
            let entryIDs = store.entriesByScope[scope].map { Array($0.keys) } ?? []
            for id in entryIDs {
                guard var entry = store.entriesByScope[scope]?[id],
                      entry.destination == destination,
                      entry.beginDiscard(revision: revision) else {
                    continue
                }
                store.entriesByScope[scope]?[id] = entry
                discarded.append(id)
            }
            return discarded.isEmpty ? .rejected : .beganDiscard(discarded)
        case .payloadGenerationReset(
            let entryID,
            let generation,
            let allocatedGeneration,
            let barrierIdle,
            let producerLive
        ):
            guard let entry = store.entriesByScope[scope]?[entryID],
                  entry.lifecycle.phase == .active,
                  barrierIdle,
                  producerLive,
                  generation == entry.currentGeneration,
                  allocatedGeneration > generation else {
                return .rejected
            }
            let operationKeys = entry.operationKeys
                .filter { $0.generation == generation }
                .sorted { $0.operationID.rawValue < $1.operationID.rawValue }
            // Reset always crosses payload, operation, resource, and hi-lo
            // durability authorities. Classify it here; only the durability
            // planner may publish the generation change.
            return .requiresDurableGenerationReset(operationKeys)
        case .payloadEntryDiscarded(let entryID, let revision):
            guard var entry = store.entriesByScope[scope]?[entryID],
                  entry.beginDiscard(revision: revision) else { return .rejected }
            store.entriesByScope[scope]?[entryID] = entry
            return .beganDiscard([entryID])
        }
    }
}
