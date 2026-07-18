import Foundation

// MARK: - Hi-lo durable identity allocation

/// Pure model of a durable hi-lo allocator. Reserving a block advances the
/// persisted high watermark before any value in that block is returned. A
/// relaunched allocator starts after the persisted watermark and intentionally
/// skips unused values from the previous process.
public struct GaryxDurableHiLoAllocator: Equatable, Codable, Sendable {
    public let blockSize: UInt64
    public private(set) var persistedHighWatermark: UInt64
    public private(set) var nextValue: UInt64
    public private(set) var blockUpperBound: UInt64
    public private(set) var durableReservationCount: UInt64

    public init(persistedHighWatermark: UInt64 = 0, blockSize: UInt64 = 32) {
        precondition(blockSize > 0)
        self.blockSize = blockSize
        self.persistedHighWatermark = persistedHighWatermark
        nextValue = persistedHighWatermark + 1
        blockUpperBound = persistedHighWatermark
        durableReservationCount = 0
    }

    public mutating func allocate() -> UInt64 {
        if nextValue > blockUpperBound {
            let previousHigh = persistedHighWatermark
            persistedHighWatermark &+= blockSize
            nextValue = previousHigh + 1
            blockUpperBound = persistedHighWatermark
            durableReservationCount &+= 1
        }
        let allocated = nextValue
        nextValue &+= 1
        return allocated
    }
}

// MARK: - Provisional reservation ledger

public struct GaryxReservationLedgerKey: Hashable, Codable, Sendable {
    public let scope: GaryxGatewayScope
    public let entryID: GaryxComposerPayloadEntryID
    public let reservationID: GaryxSendReservationID

    public init(
        scope: GaryxGatewayScope,
        entryID: GaryxComposerPayloadEntryID,
        reservationID: GaryxSendReservationID
    ) {
        self.scope = scope
        self.entryID = entryID
        self.reservationID = reservationID
    }
}

public enum GaryxReservationTerminalOutcome: String, Codable, Sendable {
    case committed
    case revoked
}

public struct GaryxReservationTargetMapping: Equatable, Codable, Sendable {
    public let entryID: GaryxComposerPayloadEntryID
    public let generation: UInt64

    public init(entryID: GaryxComposerPayloadEntryID, generation: UInt64) {
        self.entryID = entryID
        self.generation = generation
    }
}

public struct GaryxProvisionalReservationLedger: Equatable, Codable, Sendable {
    public let key: GaryxReservationLedgerKey
    public let envelopeGeneration: UInt64
    public let followupGeneration: UInt64
    public private(set) var terminalOutcome: GaryxReservationTerminalOutcome?
    public private(set) var targetMapping: GaryxReservationTargetMapping?

    public init(
        key: GaryxReservationLedgerKey,
        envelopeGeneration: UInt64,
        followupGeneration: UInt64
    ) {
        precondition(followupGeneration > envelopeGeneration)
        self.key = key
        self.envelopeGeneration = envelopeGeneration
        self.followupGeneration = followupGeneration
        terminalOutcome = nil
        targetMapping = nil
    }

    @discardableResult
    public mutating func settle(
        _ outcome: GaryxReservationTerminalOutcome,
        targetGeneration: UInt64
    ) -> Bool {
        var next = self
        switch outcome {
        case .committed:
            guard next.synthesizeTerminalOutcome(.committed),
                  next.persistTargetMapping(targetGeneration) else { return false }
        case .revoked:
            guard next.synthesizeTerminalOutcome(.revoked),
                  next.persistTargetMapping(targetGeneration) else { return false }
        }
        self = next
        return true
    }

    mutating func synthesizeTerminalOutcome(
        _ outcome: GaryxReservationTerminalOutcome
    ) -> Bool {
        guard terminalOutcome == nil, targetMapping == nil else { return false }
        terminalOutcome = outcome
        return true
    }

    mutating func persistTargetMapping(_ targetGeneration: UInt64) -> Bool {
        guard targetMapping == nil, let terminalOutcome else { return false }
        switch terminalOutcome {
        case .committed:
            guard targetGeneration == followupGeneration else { return false }
        case .revoked:
            guard targetGeneration > followupGeneration else { return false }
        }
        targetMapping = GaryxReservationTargetMapping(
            entryID: key.entryID,
            generation: targetGeneration
        )
        return true
    }
}

public enum GaryxSyntheticRecoveryStep: String, CaseIterable, Codable, Sendable {
    case synthesizeRevokedOutcome
    case allocateMergeGeneration
    case migratePayloadAndConflictSet
    case updateOperationManifests
    case persistTargetMapping
}

/// Tracks the required ordering ledger -> durable descendants -> network.
public struct GaryxReservationAdmissionTracker: Equatable, Sendable {
    public private(set) var ledgerDurable: Bool
    public private(set) var durableDescendantCount: Int
    public private(set) var networkAttempted: Bool

    public init() {
        ledgerDurable = false
        durableDescendantCount = 0
        networkAttempted = false
    }

    public mutating func persistLedger() { ledgerDurable = true }

    @discardableResult
    public mutating func persistDescendant() -> Bool {
        guard ledgerDurable else { return false }
        durableDescendantCount += 1
        return true
    }

    @discardableResult
    public mutating func crossNetworkBoundary() -> Bool {
        guard ledgerDurable else { return false }
        networkAttempted = true
        return true
    }
}

// MARK: - Delivery record and evidence

public struct GaryxDeliveryEnvelope: Equatable, Codable, Sendable {
    public let text: String
    public let attachmentIDs: [GaryxAttachmentID]
    public let generation: UInt64
    public let clientIntentID: String

    public init(
        text: String,
        attachmentIDs: [GaryxAttachmentID],
        generation: UInt64,
        clientIntentID: String
    ) {
        precondition(!clientIntentID.isEmpty)
        self.text = text
        self.attachmentIDs = attachmentIDs
        self.generation = generation
        self.clientIntentID = clientIntentID
    }

    public var estimatedBytes: Int {
        text.utf8.count + attachmentIDs.reduce(0) { $0 + $1.rawValue.utf8.count }
    }
}

public enum GaryxDeliveryRecordPhase: String, CaseIterable, Codable, Sendable {
    case notDispatched
    case transportAttempted
    case ambiguous
    case acknowledged
    case cancelledByDiscard
    case evidence
    case terminalEvidence
    case abandoned
    case supersededByDuplicate

    public var isTerminalOrEvidence: Bool {
        switch self {
        case .acknowledged, .cancelledByDiscard, .evidence, .terminalEvidence,
             .abandoned, .supersededByDuplicate:
            true
        case .notDispatched, .transportAttempted, .ambiguous:
            false
        }
    }

    var isSettledForIdentityDiscard: Bool {
        switch self {
        case .cancelledByDiscard, .evidence, .terminalEvidence,
             .abandoned, .supersededByDuplicate:
            true
        case .notDispatched, .transportAttempted, .ambiguous, .acknowledged:
            false
        }
    }
}

public enum GaryxDeliveryEvidence: String, Codable, Sendable {
    case none
    case transportAttempted
    case serverAcknowledged
}

public enum GaryxDeliveryUserDisposition: String, Codable, Sendable {
    case none
    case restoredToDraft
    case resentAsDuplicate
    case scopeRevoked
    case payloadDiscarded
}

public struct GaryxDeliveryRecord: Equatable, Codable, Sendable {
    public let id: GaryxDeliveryRecordID
    public let scope: GaryxGatewayScope
    public let entryID: GaryxComposerPayloadEntryID
    public let reservationID: GaryxSendReservationID
    public let correlationID: String
    public private(set) var envelope: GaryxDeliveryEnvelope?
    public private(set) var phase: GaryxDeliveryRecordPhase
    public private(set) var evidence: GaryxDeliveryEvidence
    public private(set) var userDisposition: GaryxDeliveryUserDisposition
    public private(set) var duplicateRecordID: GaryxDeliveryRecordID?

    public init(
        id: GaryxDeliveryRecordID,
        scope: GaryxGatewayScope,
        entryID: GaryxComposerPayloadEntryID,
        reservationID: GaryxSendReservationID,
        correlationID: String,
        envelope: GaryxDeliveryEnvelope
    ) {
        precondition(!correlationID.isEmpty)
        self.id = id
        self.scope = scope
        self.entryID = entryID
        self.reservationID = reservationID
        self.correlationID = correlationID
        self.envelope = envelope
        phase = .notDispatched
        evidence = .none
        userDisposition = .none
        duplicateRecordID = nil
    }

    @discardableResult
    public mutating func markTransportAttempted() -> Bool {
        guard phase == .notDispatched else { return false }
        phase = .transportAttempted
        evidence = .transportAttempted
        return true
    }

    @discardableResult
    public mutating func markAmbiguous() -> Bool {
        guard phase == .transportAttempted else { return false }
        phase = .ambiguous
        return true
    }

    public mutating func recordServerAcknowledgement() {
        evidence = .serverAcknowledged
        if userDisposition == .none, phase != .terminalEvidence {
            phase = .acknowledged
        }
        envelope = nil
    }

    @discardableResult
    fileprivate mutating func restoreToDraftAfterConflictAdmission() -> GaryxDeliveryEnvelope? {
        guard phase == .ambiguous, userDisposition == .none else { return nil }
        let restored = envelope
        userDisposition = .restoredToDraft
        phase = .abandoned
        envelope = nil
        return restored
    }

    @discardableResult
    public mutating func resendAsDuplicate(
        newRecordID: GaryxDeliveryRecordID,
        newClientIntentID: String
    ) -> GaryxDeliveryEnvelope? {
        guard phase == .ambiguous,
              userDisposition == .none,
              let envelope,
              newRecordID != id,
              !newClientIntentID.isEmpty,
              newClientIntentID != envelope.clientIntentID else {
            return nil
        }
        let duplicate = GaryxDeliveryEnvelope(
            text: envelope.text,
            attachmentIDs: envelope.attachmentIDs,
            generation: envelope.generation,
            clientIntentID: newClientIntentID
        )
        userDisposition = .resentAsDuplicate
        duplicateRecordID = newRecordID
        phase = .supersededByDuplicate
        self.envelope = nil
        return duplicate
    }

    /// Discard settlement is a per-record CAS independent of the current send
    /// barrier phase.
    public mutating func settleForDiscard() {
        switch phase {
        case .notDispatched:
            phase = .cancelledByDiscard
            userDisposition = .payloadDiscarded
            envelope = nil
        case .transportAttempted, .ambiguous:
            phase = .evidence
            userDisposition = .payloadDiscarded
            envelope = nil
        case .acknowledged:
            phase = .terminalEvidence
            envelope = nil
        case .cancelledByDiscard, .evidence, .terminalEvidence,
             .abandoned, .supersededByDuplicate:
            envelope = nil
        }
    }

    public mutating func settleForScopeRevoke() {
        settleForDiscard()
        if userDisposition == .none || userDisposition == .payloadDiscarded {
            userDisposition = .scopeRevoked
        }
    }

    /// Durable delivery rows are a monotonic evidence ledger. A writer may
    /// advance transport/evidence state or clear the envelope, but it may not
    /// publish an older snapshot over a concurrently acknowledged record.
    func durablyAdvances(from previous: Self) -> Bool {
        guard id == previous.id,
              scope == previous.scope,
              entryID == previous.entryID,
              reservationID == previous.reservationID,
              correlationID == previous.correlationID,
              Self.phaseCanAdvance(from: previous.phase, to: phase),
              Self.evidenceRank(evidence) >= Self.evidenceRank(previous.evidence),
              Self.userDispositionCanAdvance(
                  from: previous.userDisposition,
                  to: userDisposition
              ),
              Self.envelopeCanAdvance(from: previous.envelope, to: envelope),
              previous.duplicateRecordID == nil
                  ? true
                  : duplicateRecordID == previous.duplicateRecordID else {
            return false
        }
        return true
    }

    private static func envelopeCanAdvance(
        from previous: GaryxDeliveryEnvelope?,
        to next: GaryxDeliveryEnvelope?
    ) -> Bool {
        guard let previous else { return next == nil }
        return next == nil || next == previous
    }

    private static func phaseCanAdvance(
        from previous: GaryxDeliveryRecordPhase,
        to next: GaryxDeliveryRecordPhase
    ) -> Bool {
        if previous == next { return true }
        switch (previous, next) {
        case (.notDispatched, .transportAttempted),
             (.notDispatched, .acknowledged),
             (.notDispatched, .cancelledByDiscard),
             (.transportAttempted, .ambiguous),
             (.transportAttempted, .acknowledged),
             (.transportAttempted, .evidence),
             (.ambiguous, .acknowledged),
             (.ambiguous, .evidence),
             (.ambiguous, .abandoned),
             (.ambiguous, .supersededByDuplicate),
             (.acknowledged, .terminalEvidence):
            return true
        default:
            return false
        }
    }

    private static func evidenceRank(_ evidence: GaryxDeliveryEvidence) -> Int {
        switch evidence {
        case .none: 0
        case .transportAttempted: 1
        case .serverAcknowledged: 2
        }
    }

    private static func userDispositionCanAdvance(
        from previous: GaryxDeliveryUserDisposition,
        to next: GaryxDeliveryUserDisposition
    ) -> Bool {
        if previous == next { return true }
        switch (previous, next) {
        case (.none, _), (.payloadDiscarded, .scopeRevoked):
            return true
        default:
            return false
        }
    }

    public var persistentTombstoneEstimatedBytes: Int? {
        guard phase.isTerminalOrEvidence, envelope == nil else { return nil }
        return id.rawValue.utf8.count
            + scope.identity.utf8.count
            + entryID.rawValue.utf8.count
            + correlationID.utf8.count
            + 64
    }
}

public enum GaryxDeliveryDraftRecoveryDisposition: Equatable, Sendable {
    case restored(GaryxDeliveryEnvelope)
    case rejectedNotAmbiguous
    case rejectedConflictScope
    case rejectedConflictDurability
}

public enum GaryxDeliveryDraftRecoveryReducer {
    /// Atomic value reducer for the ambiguous "restore to draft" exit. The
    /// original record is not terminalized unless durable conflict membership
    /// can be admitted in the same transaction.
    public static func restore(
        record: inout GaryxDeliveryRecord,
        conflictSet: inout GaryxPayloadConflictSet,
        candidate: GaryxPayloadConflictCandidate,
        membershipDurabilityAvailable: Bool
    ) -> GaryxDeliveryDraftRecoveryDisposition {
        guard record.phase == .ambiguous, record.userDisposition == .none else {
            return .rejectedNotAmbiguous
        }
        guard conflictSet.scope == record.scope else { return .rejectedConflictScope }
        var nextRecord = record
        var nextConflictSet = conflictSet
        guard nextConflictSet.admitCandidate(
            candidate,
            membershipDurabilityAvailable: membershipDurabilityAvailable
        ) else {
            return .rejectedConflictDurability
        }
        guard let envelope = nextRecord.restoreToDraftAfterConflictAdmission() else {
            return .rejectedNotAmbiguous
        }
        record = nextRecord
        conflictSet = nextConflictSet
        return .restored(envelope)
    }
}

public enum GaryxDeliveryEvidenceIngressDisposition: Equatable, Sendable {
    case updated(GaryxDeliveryRecordID)
    case rejectedAuthenticationSource
    case rejectedPhase
    case ambiguousCorrelation
    case unknownCorrelation
}

public enum GaryxDeliveryEvidenceIngress {
    /// The API intentionally accepts no message body. It can update correlation
    /// evidence but cannot inject content into composer/domain state.
    public static func acknowledge(
        correlationID: String,
        authenticatedScope: GaryxGatewayScope,
        records: inout [GaryxDeliveryRecordID: GaryxDeliveryRecord]
    ) -> GaryxDeliveryEvidenceIngressDisposition {
        let matching = records.filter {
            $0.value.correlationID == correlationID && $0.value.scope == authenticatedScope
        }
        guard !matching.isEmpty else {
            if records.values.contains(where: { $0.correlationID == correlationID }) {
                return .rejectedAuthenticationSource
            }
            return .unknownCorrelation
        }
        guard matching.count == 1, let (id, record) = matching.first else {
            return .ambiguousCorrelation
        }
        switch record.phase {
        case .transportAttempted, .ambiguous, .evidence, .abandoned, .supersededByDuplicate:
            break
        case .acknowledged, .terminalEvidence:
            return .updated(id)
        case .notDispatched, .cancelledByDiscard:
            return .rejectedPhase
        }
        records[id]?.recordServerAcknowledgement()
        return .updated(id)
    }
}

public struct GaryxDeliveryQuota: Equatable, Codable, Sendable {
    public static let perScopeRecordLimit = 64
    public static let globalRecordLimit = 256

    public var nonTerminalByScope: [GaryxGatewayScope: Int]
    public var nonTerminalGlobal: Int
    public var payloadBytesUsed: Int
    public var payloadByteLimit: Int

    public init(
        nonTerminalByScope: [GaryxGatewayScope: Int] = [:],
        nonTerminalGlobal: Int = 0,
        payloadBytesUsed: Int = 0,
        payloadByteLimit: Int = 64 * 1024 * 1024
    ) {
        self.nonTerminalByScope = nonTerminalByScope
        self.nonTerminalGlobal = nonTerminalGlobal
        self.payloadBytesUsed = payloadBytesUsed
        self.payloadByteLimit = payloadByteLimit
    }

    /// Rebuilds admission accounting exclusively from durable records after a
    /// relaunch. Terminal/evidence records remain bounded correlation state,
    /// but no longer consume non-terminal or envelope quota.
    public init(
        rebuilding records: [GaryxDeliveryRecord],
        payloadByteLimit: Int = 64 * 1024 * 1024
    ) {
        var byScope: [GaryxGatewayScope: Int] = [:]
        var global = 0
        var payloadBytes = 0
        for record in records {
            if !record.phase.isTerminalOrEvidence {
                byScope[record.scope, default: 0] += 1
                global += 1
            }
            payloadBytes += record.envelope?.estimatedBytes ?? 0
        }
        self.init(
            nonTerminalByScope: byScope,
            nonTerminalGlobal: global,
            payloadBytesUsed: payloadBytes,
            payloadByteLimit: payloadByteLimit
        )
    }

    public func canSeal(scope: GaryxGatewayScope, envelopeBytes: Int) -> Bool {
        (nonTerminalByScope[scope] ?? 0) < Self.perScopeRecordLimit
            && nonTerminalGlobal < Self.globalRecordLimit
            && payloadBytesUsed + envelopeBytes <= payloadByteLimit
    }
}

public struct GaryxPersistentTombstoneBudget: Equatable, Codable, Sendable {
    public let countLimit: Int
    public let byteLimit: Int

    public init(countLimit: Int = 4_096, byteLimit: Int = 4 * 1024 * 1024) {
        precondition(countLimit >= 0 && byteLimit >= 0)
        self.countLimit = countLimit
        self.byteLimit = byteLimit
    }

    public func admits(count: Int, bytes: Int) -> Bool {
        count >= 0 && bytes >= 0 && count <= countLimit && bytes <= byteLimit
    }
}

public struct GaryxPersistentTombstoneUsage: Equatable, Codable, Sendable {
    public let correlationCount: Int
    public let correlationBytes: Int
    public let discardFinalizationCount: Int
    public let discardFinalizationBytes: Int

    public init(
        correlationCount: Int = 0,
        correlationBytes: Int = 0,
        discardFinalizationCount: Int = 0,
        discardFinalizationBytes: Int = 0
    ) {
        precondition(
            correlationCount >= 0
                && correlationBytes >= 0
                && discardFinalizationCount >= 0
                && discardFinalizationBytes >= 0
        )
        self.correlationCount = correlationCount
        self.correlationBytes = correlationBytes
        self.discardFinalizationCount = discardFinalizationCount
        self.discardFinalizationBytes = discardFinalizationBytes
    }

    public var count: Int { correlationCount + discardFinalizationCount }
    public var bytes: Int { correlationBytes + discardFinalizationBytes }
}

public enum GaryxGatewayScopeSettlementKind: Equatable, Sendable {
    case suspend
    case revoke
}

public enum GaryxSealedBarrierSettlementDecision: Equatable, Sendable {
    case durableCommit
    case revoke
}

public enum GaryxScopeBarrierSettlementAction: Equatable, Sendable {
    case durableCommitBarrier
    case revokeBarrier
    case returnBarrierToIdle
    case suspendScope
    case revokeScope
}

public enum GaryxScopeBarrierSettlementPlan: Equatable, Sendable {
    case ready([GaryxScopeBarrierSettlementAction])
    case awaitingSealedBarrierDecision
}

/// Decision table for a scope event racing a send barrier. Scope mutation is
/// always the final action; a sealed barrier must publish its terminal outcome
/// and return idle before the scope is suspended or revoked.
public enum GaryxScopeBarrierSettlementPlanner {
    public static func plan(
        barrierPhase: GaryxSendCommitBarrierPhase,
        scopeSettlement: GaryxGatewayScopeSettlementKind,
        sealedDecision: GaryxSealedBarrierSettlementDecision? = nil
    ) -> GaryxScopeBarrierSettlementPlan {
        let scopeAction: GaryxScopeBarrierSettlementAction = switch scopeSettlement {
        case .suspend: .suspendScope
        case .revoke: .revokeScope
        }
        switch barrierPhase {
        case .idle:
            return .ready([scopeAction])
        case .sealed:
            guard let sealedDecision else { return .awaitingSealedBarrierDecision }
            let terminalAction: GaryxScopeBarrierSettlementAction = switch sealedDecision {
            case .durableCommit: .durableCommitBarrier
            case .revoke: .revokeBarrier
            }
            return .ready([terminalAction, .returnBarrierToIdle, scopeAction])
        case .durableCommitted, .revoked:
            return .ready([.returnBarrierToIdle, scopeAction])
        }
    }
}

// MARK: - Send commit barrier

public enum GaryxSendCommitBarrierPhase: String, CaseIterable, Codable, Sendable {
    case idle
    case sealed
    case durableCommitted
    case revoked
}

public enum GaryxSendBarrierSealDisposition: Equatable, Sendable {
    case sealed
    case payloadPreparing
    case deliveryBackpressure
    case rejectedLifecycle
    case rejectedProducerPhase
    case busy
}

public struct GaryxSendBarrierSettlement: Equatable, Sendable {
    public let terminalOutcome: GaryxReservationTerminalOutcome
    public let followupGeneration: UInt64
    public let followupText: String
    public let followupAttachmentIDs: [GaryxAttachmentID]
    public let deliveryRecord: GaryxDeliveryRecord?
}

public struct GaryxSendCommitBarrier: Equatable, Codable, Sendable {
    public let entryID: GaryxComposerPayloadEntryID
    public let scope: GaryxGatewayScope
    public let payloadLifecycle: GaryxPayloadLifecycleCapture
    public private(set) var phase: GaryxSendCommitBarrierPhase
    public private(set) var reservationID: GaryxSendReservationID?
    public private(set) var envelopeGeneration: UInt64?
    public private(set) var followupGeneration: UInt64?
    public private(set) var envelopeText: String?
    public private(set) var envelopeAttachmentIDs: [GaryxAttachmentID]
    public private(set) var envelopeClientIntentID: String?
    public private(set) var provisionalFollowupText: String
    public private(set) var provisionalFollowupAttachmentIDs: [GaryxAttachmentID]

    public init(
        entryID: GaryxComposerPayloadEntryID,
        scope: GaryxGatewayScope,
        payloadLifecycle: GaryxPayloadLifecycleCapture
    ) {
        self.entryID = entryID
        self.scope = scope
        self.payloadLifecycle = payloadLifecycle
        phase = .idle
        reservationID = nil
        envelopeGeneration = nil
        followupGeneration = nil
        envelopeText = nil
        envelopeAttachmentIDs = []
        envelopeClientIntentID = nil
        provisionalFollowupText = ""
        provisionalFollowupAttachmentIDs = []
    }

    @discardableResult
    public mutating func seal(
        reservationID: GaryxSendReservationID,
        envelope: GaryxDeliveryEnvelope,
        followupGeneration: UInt64,
        readiness: GaryxComposerSendReadiness,
        quota: GaryxDeliveryQuota,
        producerPhase: GaryxProducerFinalizationPhase,
        lifecycle: GaryxPayloadLifecycleSnapshot
    ) -> GaryxSendBarrierSealDisposition {
        guard payloadLifecycle.isAdmitted(by: lifecycle) else { return .rejectedLifecycle }
        guard producerPhase == .live else { return .rejectedProducerPhase }
        guard phase == .idle else { return .busy }
        guard readiness == .ready else { return .payloadPreparing }
        guard quota.canSeal(scope: scope, envelopeBytes: envelope.estimatedBytes) else {
            return .deliveryBackpressure
        }
        precondition(followupGeneration > envelope.generation)
        phase = .sealed
        self.reservationID = reservationID
        envelopeGeneration = envelope.generation
        self.followupGeneration = followupGeneration
        envelopeText = envelope.text
        envelopeAttachmentIDs = envelope.attachmentIDs
        envelopeClientIntentID = envelope.clientIntentID
        provisionalFollowupText = ""
        provisionalFollowupAttachmentIDs = []
        return .sealed
    }

    @discardableResult
    public mutating func replaceProvisionalText(
        _ text: String,
        lifecycle: GaryxPayloadLifecycleSnapshot
    ) -> Bool {
        guard phase == .sealed, payloadLifecycle.isAdmitted(by: lifecycle) else { return false }
        provisionalFollowupText = text
        return true
    }

    @discardableResult
    public mutating func addProvisionalAttachment(
        _ id: GaryxAttachmentID,
        lifecycle: GaryxPayloadLifecycleSnapshot
    ) -> Bool {
        guard phase == .sealed, payloadLifecycle.isAdmitted(by: lifecycle) else { return false }
        provisionalFollowupAttachmentIDs.append(id)
        return true
    }

    public mutating func durableCommit(
        deliveryID: GaryxDeliveryRecordID,
        correlationID: String,
        clientIntentID: String,
        lifecycle: GaryxPayloadLifecycleSnapshot
    ) -> GaryxSendBarrierSettlement? {
        guard payloadLifecycle.isAdmitted(by: lifecycle),
              phase == .sealed,
              let reservationID,
              let envelopeGeneration,
              let followupGeneration,
              let envelopeText,
              let envelopeClientIntentID,
              clientIntentID == envelopeClientIntentID else {
            return nil
        }
        phase = .durableCommitted
        let envelope = GaryxDeliveryEnvelope(
            text: envelopeText,
            attachmentIDs: envelopeAttachmentIDs,
            generation: envelopeGeneration,
            clientIntentID: envelopeClientIntentID
        )
        let delivery = GaryxDeliveryRecord(
            id: deliveryID,
            scope: scope,
            entryID: entryID,
            reservationID: reservationID,
            correlationID: correlationID,
            envelope: envelope
        )
        return GaryxSendBarrierSettlement(
            terminalOutcome: .committed,
            followupGeneration: followupGeneration,
            followupText: provisionalFollowupText,
            followupAttachmentIDs: provisionalFollowupAttachmentIDs,
            deliveryRecord: delivery
        )
    }

    public mutating func revoke(
        mergeGeneration: UInt64,
        lifecycle: GaryxPayloadLifecycleSnapshot
    ) -> GaryxSendBarrierSettlement? {
        guard payloadLifecycle.isAdmitted(by: lifecycle),
              phase == .sealed,
              let followupGeneration,
              mergeGeneration > followupGeneration else {
            return nil
        }
        phase = .revoked
        return GaryxSendBarrierSettlement(
            terminalOutcome: .revoked,
            followupGeneration: mergeGeneration,
            followupText: (envelopeText ?? "") + provisionalFollowupText,
            followupAttachmentIDs: envelopeAttachmentIDs + provisionalFollowupAttachmentIDs,
            deliveryRecord: nil
        )
    }

    /// Dedicated discard settlement path; ordinary callers cannot use it to
    /// bypass lifecycle admission.
    fileprivate mutating func forceRevokeForDiscard() {
        guard phase == .sealed else { return }
        phase = .revoked
        envelopeText = nil
        envelopeAttachmentIDs = []
        envelopeClientIntentID = nil
        provisionalFollowupText = ""
        provisionalFollowupAttachmentIDs = []
    }

    public mutating func returnToIdle() {
        guard phase == .durableCommitted || phase == .revoked else { return }
        phase = .idle
        reservationID = nil
        envelopeGeneration = nil
        followupGeneration = nil
        envelopeText = nil
        envelopeAttachmentIDs = []
        envelopeClientIntentID = nil
        provisionalFollowupText = ""
        provisionalFollowupAttachmentIDs = []
    }
}

// MARK: - Multi-stage create delivery

public enum GaryxCreateDeliveryPhase: String, Codable, Sendable {
    case createPending
    case threadCreated
    case bindingCompleted
    case chatStartAttempted
    case acknowledged
    case ambiguous
}

public struct GaryxCreateDeliveryKey: Hashable, Codable, Sendable {
    public let scope: GaryxGatewayScope
    public let createIntentID: String

    public init(scope: GaryxGatewayScope, createIntentID: String) {
        precondition(!createIntentID.isEmpty)
        self.scope = scope
        self.createIntentID = createIntentID
    }
}

public struct GaryxCreateDeliveryState: Equatable, Codable, Sendable {
    public let key: GaryxCreateDeliveryKey
    public private(set) var threadID: String?
    public private(set) var phase: GaryxCreateDeliveryPhase
    public private(set) var ambiguousAfter: GaryxCreateDeliveryPhase?
    public private(set) var userDisposition: GaryxCreateAmbiguousDisposition

    public init(scope: GaryxGatewayScope, createIntentID: String) {
        key = GaryxCreateDeliveryKey(scope: scope, createIntentID: createIntentID)
        threadID = nil
        phase = .createPending
        ambiguousAfter = nil
        userDisposition = .none
    }

    public var scope: GaryxGatewayScope { key.scope }
    public var createIntentID: String { key.createIntentID }

    public mutating func created(threadID: String) {
        guard phase == .createPending, !threadID.isEmpty else { return }
        self.threadID = threadID
        phase = .threadCreated
    }

    public mutating func bound() {
        guard phase == .threadCreated else { return }
        phase = .bindingCompleted
    }

    public mutating func chatStartAttempted() {
        guard phase == .threadCreated || phase == .bindingCompleted else { return }
        phase = .chatStartAttempted
    }

    public mutating func responseLost() {
        guard phase != .acknowledged, phase != .ambiguous else { return }
        ambiguousAfter = phase
        phase = .ambiguous
    }

    public mutating func acknowledged() {
        guard phase == .chatStartAttempted
                || (phase == .ambiguous && ambiguousAfter == .chatStartAttempted) else {
            return
        }
        phase = .acknowledged
        ambiguousAfter = nil
    }

    @discardableResult
    public mutating func restoreToDraft() -> Bool {
        guard phase == .ambiguous, userDisposition == .none else { return false }
        userDisposition = .restoredToDraft
        return true
    }

    public mutating func rebuildWithDuplicateRisk(
        newCreateIntentID: String
    ) -> GaryxCreateDeliveryState? {
        guard phase == .ambiguous,
              userDisposition == .none,
              !newCreateIntentID.isEmpty,
              newCreateIntentID != createIntentID else {
            return nil
        }
        userDisposition = .rebuildMayCreateDuplicateThread
        return GaryxCreateDeliveryState(scope: scope, createIntentID: newCreateIntentID)
    }
}

public enum GaryxCreateAmbiguousDisposition: String, Codable, Sendable {
    case none
    case restoredToDraft
    case rebuildMayCreateDuplicateThread
}

// MARK: - Discard convergence

public enum GaryxSessionDescendantPhase: String, Codable, Sendable {
    case live
    case finalizing
    case closePendingAck
    case retired
}

public struct GaryxSessionDescendantKey: Hashable, Codable, Sendable {
    public let token: GaryxPayloadLifecycleToken
    public let sessionID: GaryxComposerInputSessionID
    public let epoch: UInt64

    public init(
        token: GaryxPayloadLifecycleToken,
        sessionID: GaryxComposerInputSessionID,
        epoch: UInt64
    ) {
        self.token = token
        self.sessionID = sessionID
        self.epoch = epoch
    }
}

public struct GaryxSessionDescendant: Equatable, Codable, Sendable {
    public let key: GaryxSessionDescendantKey
    /// Alias is retained only for event routing; settlement membership uses key.token.
    public let composerKey: GaryxComposerKey
    public let phase: GaryxSessionDescendantPhase
    public let finalSequence: UInt64?

    public init(
        key: GaryxSessionDescendantKey,
        composerKey: GaryxComposerKey,
        phase: GaryxSessionDescendantPhase,
        finalSequence: UInt64?
    ) {
        self.key = key
        self.composerKey = composerKey
        self.phase = phase
        self.finalSequence = finalSequence
    }
}

public struct GaryxDiscardFinalizationTombstoneKey: Hashable, Codable, Sendable {
    public let token: GaryxPayloadLifecycleToken
    public let discardRevision: UInt64
    public let sessionID: GaryxComposerInputSessionID
    public let epoch: UInt64

    public init(
        token: GaryxPayloadLifecycleToken,
        discardRevision: UInt64,
        sessionID: GaryxComposerInputSessionID,
        epoch: UInt64
    ) {
        self.token = token
        self.discardRevision = discardRevision
        self.sessionID = sessionID
        self.epoch = epoch
    }
}

public enum GaryxDiscardFinalizationDisposition: String, Codable, Sendable {
    case finalizerTerminated
    case closePendingAckConverted
}

/// Terminal metadata intentionally contains no text, attachment, or file path.
public struct GaryxDiscardFinalizationTombstone: Equatable, Codable, Sendable {
    public let key: GaryxDiscardFinalizationTombstoneKey
    public let finalSequence: UInt64?
    public let disposition: GaryxDiscardFinalizationDisposition

    public init(
        key: GaryxDiscardFinalizationTombstoneKey,
        finalSequence: UInt64?,
        disposition: GaryxDiscardFinalizationDisposition
    ) {
        self.key = key
        self.finalSequence = finalSequence
        self.disposition = disposition
    }

    public var estimatedBytes: Int {
        key.token.entryID.rawValue.utf8.count
            + key.token.nonce.utf8.count
            + key.sessionID.rawValue.utf8.count
            + 64
    }
}

public enum GaryxLateCloseAcknowledgementDisposition: Equatable, Sendable {
    case rejectedTombstoned
    case rejectedUnknownToken
}

public struct GaryxPayloadDiscardConvergence: Equatable, Codable, Sendable {
    public private(set) var lifecycle: GaryxPayloadLifecycleRecord
    public private(set) var barrier: GaryxSendCommitBarrier
    public private(set) var sessions: [GaryxSessionDescendantKey: GaryxSessionDescendant]
    public private(set) var deliveries: [GaryxDeliveryRecordID: GaryxDeliveryRecord]
    public private(set) var operations: [GaryxOperationCapabilityKey: GaryxOperationCapability]
    public private(set) var replacements: [GaryxReplacementID: GaryxReplacementRecord]
    public private(set) var feedback: [GaryxFeedbackID: GaryxOperationFeedback]
    public private(set) var attachmentLineages: [
        GaryxAttachmentLineageID: GaryxAttachmentLineageTombstone
    ]
    public private(set) var stagedAssetIDs: Set<GaryxStagedAssetID>
    public private(set) var reservedBytes: Int
    public private(set) var tombstones: [
        GaryxDiscardFinalizationTombstoneKey: GaryxDiscardFinalizationTombstone
    ]
    public private(set) var resourcesSettled: Bool

    public init(
        lifecycle: GaryxPayloadLifecycleRecord,
        barrier: GaryxSendCommitBarrier,
        sessions: [GaryxSessionDescendantKey: GaryxSessionDescendant] = [:],
        deliveries: [GaryxDeliveryRecordID: GaryxDeliveryRecord] = [:],
        operations: [GaryxOperationCapabilityKey: GaryxOperationCapability] = [:],
        replacements: [GaryxReplacementID: GaryxReplacementRecord] = [:],
        feedback: [GaryxFeedbackID: GaryxOperationFeedback] = [:],
        attachmentLineages: [
            GaryxAttachmentLineageID: GaryxAttachmentLineageTombstone
        ] = [:],
        stagedAssetIDs: Set<GaryxStagedAssetID> = [],
        reservedBytes: Int = 0
    ) {
        precondition(lifecycle.phase == .discarding)
        self.lifecycle = lifecycle
        self.barrier = barrier
        self.sessions = sessions
        self.deliveries = deliveries
        self.operations = operations
        self.replacements = replacements
        self.feedback = feedback
        self.attachmentLineages = attachmentLineages
        self.stagedAssetIDs = stagedAssetIDs
        self.reservedBytes = reservedBytes
        tombstones = [:]
        resourcesSettled = false
    }

    public var reservationSettled: Bool { barrier.phase != .sealed }
    public var descendantsEmpty: Bool {
        !sessions.values.contains(where: {
            $0.key.token == lifecycle.token && $0.phase != .retired
        })
    }
    public var deliveriesSettled: Bool {
        deliveries.values
            .filter {
                $0.entryID == lifecycle.token.entryID && $0.scope == barrier.scope
            }
            .allSatisfy { $0.phase.isSettledForIdentityDiscard }
    }
    public var persistentTombstoneCount: Int { tombstones.count }
    public var persistentTombstoneBytes: Int {
        tombstones.values.reduce(0) { $0 + $1.estimatedBytes }
    }
    /// Reconstructs the exact alias occupancy owned by the sessions settled
    /// for this discard. Retired sessions without a tombstone were already
    /// drained before admission and contribute nothing.
    public var aliasReleases: [GaryxComposerAliasRelease] {
        guard let discardRevision = lifecycle.discardRevision else { return [] }
        return sessions.values.compactMap { session in
            guard session.key.token == lifecycle.token else { return nil }
            let tombstoneKey = GaryxDiscardFinalizationTombstoneKey(
                token: lifecycle.token,
                discardRevision: discardRevision,
                sessionID: session.key.sessionID,
                epoch: session.key.epoch
            )
            guard let tombstone = tombstones[tombstoneKey] else { return nil }
            return GaryxComposerAliasRelease(
                origin: session.composerKey,
                activeOrClosingSessions: 1,
                pendingCloseAcknowledgements: tombstone.disposition
                    == .closePendingAckConverted ? 1 : 0
            )
        }
    }

    /// Orthogonal component 1: every record uses its own phase CAS.
    public mutating func settleDeliveries() {
        let captured = deliveries
        _ = settleDeliveries(authoritativeRecords: captured)
    }

    /// Reconciles the convergence snapshot with the current durable delivery
    /// ledger before applying each discard CAS. Captured records are discovery
    /// metadata only; an absent current row is never resurrected, and current
    /// rows added after admission are included in the settlement.
    @discardableResult
    public mutating func settleDeliveries(
        authoritativeRecords: [GaryxDeliveryRecordID: GaryxDeliveryRecord]
    ) -> [GaryxDeliveryRecord] {
        let entryID = lifecycle.token.entryID
        let scope = barrier.scope
        let capturedIDs = Set(deliveries.compactMap { id, record in
            record.entryID == entryID && record.scope == scope ? id : nil
        })
        let currentIDs = Set(authoritativeRecords.compactMap { id, record in
            record.entryID == entryID && record.scope == scope ? id : nil
        })
        var settled: [GaryxDeliveryRecord] = []
        for id in capturedIDs.union(currentIDs) {
            guard var record = authoritativeRecords[id],
                  record.entryID == entryID,
                  record.scope == scope else {
                deliveries.removeValue(forKey: id)
                continue
            }
            record.settleForDiscard()
            deliveries[id] = record
            settled.append(record)
        }
        return settled
    }

    /// Orthogonal component 2: active reservation becomes revoked and all
    /// envelope/follow-up payload is cleared rather than merged into G+2.
    public mutating func settleReservation() {
        barrier.forceRevokeForDiscard()
    }

    /// Orthogonal component 3: enumerate by stable token, never ComposerKey.
    public mutating func settleSessions() {
        guard let discardRevision = lifecycle.discardRevision else { return }
        let members = sessions.values.filter { $0.key.token == lifecycle.token }
        for session in members {
            switch session.phase {
            case .live, .finalizing, .closePendingAck:
                let tombstoneKey = GaryxDiscardFinalizationTombstoneKey(
                    token: lifecycle.token,
                    discardRevision: discardRevision,
                    sessionID: session.key.sessionID,
                    epoch: session.key.epoch
                )
                tombstones[tombstoneKey] = GaryxDiscardFinalizationTombstone(
                    key: tombstoneKey,
                    finalSequence: session.finalSequence,
                    disposition: session.phase == .closePendingAck
                        ? .closePendingAckConverted
                        : .finalizerTerminated
                )
                // Retain only the payload-free retired membership until the
                // resource component runs. Its composerKey is the stable seed
                // needed to retire this Entry's alias path without deleting a
                // different Entry that happens to fan in to the same target.
                sessions[session.key] = GaryxSessionDescendant(
                    key: session.key,
                    composerKey: session.composerKey,
                    phase: .retired,
                    finalSequence: session.finalSequence
                )
            case .retired:
                continue
            }
        }
    }

    /// Identity-discard overrides terminal resource holders as well as active
    /// capabilities. Physical file deletion is represented by clearing owner IDs.
    public mutating func settleResources() {
        let entryID = lifecycle.token.entryID
        let scope = barrier.scope
        let operationKeys = operations.keys.filter {
            $0.entryID == entryID && $0.scope == scope
        }
        let replacementIDs = replacements.compactMap { id, record in
            record.entryID == entryID && record.scope == scope ? id : nil
        }
        let feedbackIDs = feedback.compactMap { id, record in
            record.entryID == entryID && record.scope == scope ? id : nil
        }
        let lineageIDs = attachmentLineages.compactMap { id, record in
            record.entryID == entryID && record.scope == scope ? id : nil
        }
        var settledAssetBytes: [GaryxStagedAssetID: Int] = [:]
        for key in operationKeys {
            if let operation = operations[key], let assetID = operation.stagedAssetID {
                settledAssetBytes[assetID] = max(settledAssetBytes[assetID] ?? 0, operation.reservedBytes)
            }
            operations[key]?.settleIdentityDiscard()
        }
        for id in replacementIDs {
            if let replacement = replacements[id] {
                settledAssetBytes[replacement.stagedAssetID] = max(
                    settledAssetBytes[replacement.stagedAssetID] ?? 0,
                    replacement.reservedBytes
                )
            }
            switch replacements[id]?.phase {
            case .pendingReplacement:
                replacements[id]?.abort()
                replacements[id]?.settle()
            case .committed:
                replacements[id]?.settle()
            case .aborted:
                replacements[id]?.settle()
            case .settled, .none:
                break
            }
        }
        for id in feedbackIDs {
            feedback[id]?.archive()
        }
        for id in lineageIDs {
            guard let feedbackID = attachmentLineages[id]?.feedbackID,
                  let terminalFeedback = feedback[feedbackID] else {
                continue
            }
            _ = attachmentLineages[id]?.release(after: terminalFeedback)
        }
        for (assetID, bytes) in settledAssetBytes where stagedAssetIDs.remove(assetID) != nil {
            reservedBytes = max(0, reservedBytes - bytes)
        }
        resourcesSettled = lineageIDs.allSatisfy {
            attachmentLineages[$0]?.phase == .released
        }
    }

    @discardableResult
    public mutating func finishToken() -> Bool {
        guard resourcesSettled else { return false }
        return lifecycle.finishDiscard(
            reservationSettled: reservationSettled,
            descendantsEmpty: descendantsEmpty,
            deliveriesSettled: deliveriesSettled
        )
    }

    public func receiveLateCloseAcknowledgement(
        sessionID: GaryxComposerInputSessionID,
        epoch: UInt64
    ) -> GaryxLateCloseAcknowledgementDisposition {
        if tombstones.keys.contains(where: {
            $0.token == lifecycle.token && $0.sessionID == sessionID && $0.epoch == epoch
        }) {
            return .rejectedTombstoned
        }
        return .rejectedUnknownToken
    }

    @discardableResult
    public mutating func garbageCollectTombstonesIfEligible() -> Bool {
        guard lifecycle.phase == .discarded,
              descendantsEmpty,
              deliveriesSettled,
              resourcesSettled else {
            return false
        }
        tombstones.removeAll()
        return true
    }
}
