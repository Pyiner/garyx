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

    public init(
        id: GaryxAttachmentID,
        stagedAssetID: GaryxStagedAssetID,
        generation: UInt64,
        byteCount: Int
    ) {
        precondition(byteCount >= 0)
        self.id = id
        self.stagedAssetID = stagedAssetID
        self.generation = generation
        self.byteCount = byteCount
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

    public mutating func addOperation(_ key: GaryxOperationCapabilityKey) {
        operationKeys.insert(key)
    }

    public mutating func removeOperation(_ key: GaryxOperationCapabilityKey) {
        operationKeys.remove(key)
    }

    public mutating func addDeliveryReference(_ id: GaryxDeliveryRecordID) {
        deliveryReferences.insert(id)
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
    public mutating func resetGeneration(
        _ generation: UInt64,
        barrierIdle: Bool,
        producerLive: Bool
    ) -> Bool {
        guard lifecycle.phase == .active,
              barrierIdle,
              producerLive,
              generation == currentGeneration else {
            return false
        }
        textByGeneration.removeValue(forKey: generation)
        attachments = attachments.filter { $0.value.generation != generation }
        operationKeys = Set(operationKeys.filter { $0.generation != generation })
        currentGeneration &+= 1
        return true
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
        guard var entry = entriesByScope[scope]?[entryID], entry.lifecycle.phase == .active else {
            return false
        }
        let stableToken = entry.lifecycle.token
        entry.promote(to: target)
        precondition(entry.lifecycle.token == stableToken, "promotion must preserve EntryID/token")
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
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) -> GaryxOperationTransitionDisposition {
        guard expectedKey == context.key else { return .rejectedKey }
        guard context.payloadLifecycle.isAdmitted(by: lifecycle) else { return .rejectedLifecycle }
        guard scopes.admitDomainEvent(from: context.key.scope) != .rejectedRevoked else {
            return .rejectedScope
        }
        guard identityValid else { return .archivedIdentityInvalid }
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
        if next == .superseded { self.supersededBy = supersededBy }
        return .applied
    }

    /// Completion's operation x scope x identity triple CAS.
    @discardableResult
    public mutating func complete(
        expectedKey: GaryxOperationCapabilityKey,
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) -> GaryxOperationTransitionDisposition {
        guard expectedKey == context.key else { return .rejectedKey }
        guard identityValid else {
            state = .cancelled
            stagedAssetID = nil
            reservedBytes = 0
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
}

public enum GaryxOperationRecoveryDecision: Equatable, Sendable {
    case cancelAndCleanStaging(erasePayload: Bool)
    case retryBeforeTransport
    case suspendInOriginPartition
    case failedRetryableWithFeedback
    case scopeRevokedEvidence
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
            return uploadAttempted ? .scopeRevokedEvidence : .cancelAndCleanStaging(erasePayload: true)
        case (.completed, .active), (.completed, .suspended):
            return .placeCompletedAndCleanStaging
        case (.completed, .revoked):
            return .scopeRevokedEvidence
        case (.failedRetryable, .active), (.failedRetryable, .suspended):
            return .preserveFailedRetryable
        case (.failedRetryable, .revoked):
            return .cancelAndCleanStaging(erasePayload: true)
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
        case .pendingReplacement:
            .abortReleaseQuotaAndDeleteProvisional
        case .committed:
            record.hasValidCommittedKey
                ? .restoreSuccessor(record.newKey!)
                : .abortReleaseQuotaAndDeleteProvisional
        case .aborted, .settled:
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
    public private(set) var pendingDecision: Bool

    public init(
        id: GaryxPayloadConflictSetID,
        scope: GaryxGatewayScope,
        candidates: [GaryxPayloadConflictCandidate] = []
    ) {
        self.id = id
        self.scope = scope
        self.candidates = candidates
        pendingDecision = !candidates.isEmpty
    }

    @discardableResult
    public mutating func admitCandidate(
        _ candidate: GaryxPayloadConflictCandidate,
        membershipDurabilityAvailable: Bool
    ) -> Bool {
        guard membershipDurabilityAvailable else { return false }
        guard !candidates.contains(where: { $0.entryID == candidate.entryID }) else { return true }
        candidates.append(candidate)
        pendingDecision = true
        return true
    }

    public mutating func resolve(entryID: GaryxComposerPayloadEntryID) {
        candidates.removeAll(where: { $0.entryID == entryID })
        pendingDecision = !candidates.isEmpty
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
        barrierIdle: Bool,
        producerLive: Bool
    )
    case payloadEntryDiscarded(GaryxComposerPayloadEntryID, revision: UInt64)
}

public enum GaryxPayloadIdentityEventDisposition: Equatable, Sendable {
    case aliasOnly
    case occurrenceOnly
    case beganDiscard([GaryxComposerPayloadEntryID])
    case generationReset
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
        case .payloadGenerationReset(let entryID, let generation, let barrierIdle, let producerLive):
            guard var entry = store.entriesByScope[scope]?[entryID],
                  entry.resetGeneration(
                      generation,
                      barrierIdle: barrierIdle,
                      producerLive: producerLive
                  ) else { return .rejected }
            store.entriesByScope[scope]?[entryID] = entry
            return .generationReset
        case .payloadEntryDiscarded(let entryID, let revision):
            guard var entry = store.entriesByScope[scope]?[entryID],
                  entry.beginDiscard(revision: revision) else { return .rejected }
            store.entriesByScope[scope]?[entryID] = entry
            return .beganDiscard([entryID])
        }
    }
}
