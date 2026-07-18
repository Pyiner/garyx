import Foundation

// MARK: - Durable composer identities

public struct GaryxComposerPayloadEntryID: RawRepresentable, Hashable, Codable, Sendable {
    public let rawValue: String

    public init(rawValue: String) {
        precondition(!rawValue.isEmpty, "composer payload entry ID must not be empty")
        self.rawValue = rawValue
    }
}

public struct GaryxComposerInputSessionID: RawRepresentable, Hashable, Codable, Sendable {
    public let rawValue: String

    public init(rawValue: String) {
        precondition(!rawValue.isEmpty, "composer input session ID must not be empty")
        self.rawValue = rawValue
    }
}

public struct GaryxSendReservationID: RawRepresentable, Hashable, Codable, Sendable {
    public let rawValue: UInt64

    public init(rawValue: UInt64) {
        precondition(rawValue > 0, "send reservation ID must be positive")
        self.rawValue = rawValue
    }
}

public struct GaryxPayloadLifecycleToken: Hashable, Codable, Sendable {
    public let entryID: GaryxComposerPayloadEntryID
    public let nonce: String

    public init(entryID: GaryxComposerPayloadEntryID, nonce: String) {
        precondition(!nonce.isEmpty, "payload lifecycle nonce must not be empty")
        self.entryID = entryID
        self.nonce = nonce
    }
}

public enum GaryxPayloadLifecyclePhase: String, Codable, Sendable {
    case active
    case discarding
    case discarded
}

public struct GaryxPayloadLifecycleSnapshot: Equatable, Codable, Sendable {
    public let token: GaryxPayloadLifecycleToken
    public let revision: UInt64
    public let phase: GaryxPayloadLifecyclePhase

    public init(
        token: GaryxPayloadLifecycleToken,
        revision: UInt64,
        phase: GaryxPayloadLifecyclePhase
    ) {
        self.token = token
        self.revision = revision
        self.phase = phase
    }
}

public struct GaryxPayloadLifecycleCapture: Equatable, Codable, Sendable {
    public let token: GaryxPayloadLifecycleToken
    public let revision: UInt64

    public init(token: GaryxPayloadLifecycleToken, revision: UInt64) {
        self.token = token
        self.revision = revision
    }

    public func isAdmitted(by snapshot: GaryxPayloadLifecycleSnapshot) -> Bool {
        snapshot.token == token && snapshot.revision == revision && snapshot.phase == .active
    }
}

public struct GaryxComposerInputSession: Equatable, Codable, Sendable {
    public let composerKey: GaryxComposerKey
    public let sessionID: GaryxComposerInputSessionID
    public let epoch: UInt64
    public let scope: GaryxGatewayScope
    public let payloadLifecycle: GaryxPayloadLifecycleCapture

    public init(
        composerKey: GaryxComposerKey,
        sessionID: GaryxComposerInputSessionID,
        epoch: UInt64,
        scope: GaryxGatewayScope,
        payloadLifecycle: GaryxPayloadLifecycleCapture
    ) {
        self.composerKey = composerKey
        self.sessionID = sessionID
        self.epoch = epoch
        self.scope = scope
        self.payloadLifecycle = payloadLifecycle
    }
}

/// Complete durable event identity. ReservationID is required while a session
/// is in a sealed reservation window.
public struct GaryxComposerInputEventIdentity: Equatable, Codable, Sendable {
    public let composerKey: GaryxComposerKey
    public let sessionID: GaryxComposerInputSessionID
    public let inputSessionEpoch: UInt64
    public let payloadGeneration: UInt64
    public let reservationID: GaryxSendReservationID?
    public let inputSequence: UInt64

    public init(
        composerKey: GaryxComposerKey,
        sessionID: GaryxComposerInputSessionID,
        inputSessionEpoch: UInt64,
        payloadGeneration: UInt64,
        reservationID: GaryxSendReservationID?,
        inputSequence: UInt64
    ) {
        self.composerKey = composerKey
        self.sessionID = sessionID
        self.inputSessionEpoch = inputSessionEpoch
        self.payloadGeneration = payloadGeneration
        self.reservationID = reservationID
        self.inputSequence = inputSequence
    }
}

// MARK: - Reservation x producer product reducer

public enum GaryxInputReservationPhase: String, CaseIterable, Codable, Sendable {
    case none
    case sealed
    case committed
    case revoked
}

public enum GaryxProducerFinalizationPhase: String, CaseIterable, Codable, Sendable {
    case live
    case finalizing
    case terminal
}

public enum GaryxInputProductTarget: Equatable, Sendable {
    case currentGeneration
    case provisionalNextGeneration
    case committedNextGeneration
    case revokedMergeGeneration
    case terminalAudit
}

/// The design's authoritative 4 x 3 table, expressed as one total function.
public enum GaryxComposerInputProductReducer {
    public static func target(
        reservation: GaryxInputReservationPhase,
        producer: GaryxProducerFinalizationPhase
    ) -> GaryxInputProductTarget {
        guard producer != .terminal else { return .terminalAudit }
        switch reservation {
        case .none:
            return .currentGeneration
        case .sealed:
            return .provisionalNextGeneration
        case .committed:
            return .committedNextGeneration
        case .revoked:
            return .revokedMergeGeneration
        }
    }
}

public enum GaryxInputProducerKind: String, Hashable, CaseIterable, Codable, Sendable {
    case markedText
    case dictation
    case scribble
}

public enum GaryxInputProducerCancellation: String, CaseIterable, Codable, Sendable {
    case sceneInactive
    case superseded
    case scopeSuspend
    case scopeRevoke
    case hostTeardown
    case transactionSettleTerminal
}

public struct GaryxInputFinalizationLease: Equatable, Codable, Sendable {
    public let sessionID: GaryxComposerInputSessionID
    public private(set) var pendingProducers: Set<GaryxInputProducerKind>
    public private(set) var terminalCancellation: GaryxInputProducerCancellation?

    public init(
        sessionID: GaryxComposerInputSessionID,
        pendingProducers: Set<GaryxInputProducerKind>
    ) {
        self.sessionID = sessionID
        self.pendingProducers = pendingProducers
        terminalCancellation = nil
    }

    public var isTerminal: Bool { pendingProducers.isEmpty }

    public mutating func producerReachedTerminal(_ producer: GaryxInputProducerKind) {
        pendingProducers.remove(producer)
    }

    public mutating func cancelAll(_ reason: GaryxInputProducerCancellation) {
        guard !pendingProducers.isEmpty else { return }
        pendingProducers.removeAll()
        terminalCancellation = reason
    }
}

public struct GaryxProducerDrainedRecord: Equatable, Codable, Sendable {
    public let sessionID: GaryxComposerInputSessionID
    public let epoch: UInt64
    public let finalSequence: UInt64
    public let bufferedText: String

    public init(
        sessionID: GaryxComposerInputSessionID,
        epoch: UInt64,
        finalSequence: UInt64,
        bufferedText: String
    ) {
        self.sessionID = sessionID
        self.epoch = epoch
        self.finalSequence = finalSequence
        self.bufferedText = bufferedText
    }
}

public struct GaryxComposerEpochSnapshot: Equatable, Codable, Sendable {
    public let sessionEpoch: UInt64
    public let payloadGeneration: UInt64
    public let text: String

    public init(sessionEpoch: UInt64, payloadGeneration: UInt64, text: String) {
        self.sessionEpoch = sessionEpoch
        self.payloadGeneration = payloadGeneration
        self.text = text
    }
}

public struct GaryxInputReservationTerminalRecord: Equatable, Codable, Sendable {
    public let reservationID: GaryxSendReservationID
    public let outcome: GaryxReservationTerminalOutcome
    public let targetGeneration: UInt64

    public init(
        reservationID: GaryxSendReservationID,
        outcome: GaryxReservationTerminalOutcome,
        targetGeneration: UInt64
    ) {
        self.reservationID = reservationID
        self.outcome = outcome
        self.targetGeneration = targetGeneration
    }
}

public enum GaryxComposerInputEventDisposition: Equatable, Sendable {
    case applied(target: GaryxInputProductTarget, generation: UInt64)
    case duplicateOrOutOfOrder
    case auditedTerminalDuplicate
    case auditedTerminalReservation
    case rejectedToken
    case rejectedScope
    case rejectedUnknownSession
    case rejectedOldGeneration
    case rejectedFutureGeneration
    case rejectedReservation
    case producerContractFault
}

public enum GaryxBeginSendInputDisposition: Equatable, Sendable {
    case sealed(envelope: String, followupGeneration: UInt64)
    case rejectedToken
    case rejectedScope
    case rejectedFinalizing
    case rejectedBusy
}

public enum GaryxInputReleaseDisposition: Equatable, Sendable {
    case released
    case rejectedToken
    case rejectedScope
    case rejectedPhase
}

public enum GaryxProducerTerminalDisposition: Equatable, Sendable {
    case stillWaiting
    case producerDrainedAwaitingReservation
    case dualTerminalCommitted
    case alreadyTerminal
    case rejectedToken
    case rejectedScope
}

public struct GaryxComposerInputReducerState: Equatable, Codable, Sendable {
    public let session: GaryxComposerInputSession
    public private(set) var reservationPhase: GaryxInputReservationPhase
    public private(set) var producerPhase: GaryxProducerFinalizationPhase
    public private(set) var currentGeneration: UInt64
    public private(set) var reservedGeneration: UInt64?
    public private(set) var revokedMergeGeneration: UInt64?
    public private(set) var activeReservationID: GaryxSendReservationID?
    public private(set) var lastAppliedSequence: UInt64
    public private(set) var finalSequence: UInt64?
    public private(set) var textByGeneration: [UInt64: String]
    public private(set) var sealedEnvelope: String?
    public private(set) var finalText: String?
    public private(set) var nextEpochSnapshot: GaryxComposerEpochSnapshot?
    public private(set) var producerDrained: GaryxProducerDrainedRecord?
    public private(set) var terminalReservations: [
        GaryxSendReservationID: GaryxInputReservationTerminalRecord
    ]
    public private(set) var closePublicationCount: Int
    public private(set) var closeAcknowledged: Bool
    public private(set) var focusClearedAtRelease: Bool
    public private(set) var canonicalPathCommittedAtRelease: Bool
    public private(set) var finalizationLease: GaryxInputFinalizationLease?
    public private(set) var faultCount: Int

    public init(
        session: GaryxComposerInputSession,
        payloadGeneration: UInt64,
        initialText: String = "",
        lastAppliedSequence: UInt64 = 0
    ) {
        self.session = session
        reservationPhase = .none
        producerPhase = .live
        currentGeneration = payloadGeneration
        reservedGeneration = nil
        revokedMergeGeneration = nil
        activeReservationID = nil
        self.lastAppliedSequence = lastAppliedSequence
        finalSequence = nil
        textByGeneration = initialText.isEmpty ? [:] : [payloadGeneration: initialText]
        sealedEnvelope = nil
        finalText = nil
        nextEpochSnapshot = nil
        producerDrained = nil
        terminalReservations = [:]
        closePublicationCount = 0
        closeAcknowledged = false
        focusClearedAtRelease = false
        canonicalPathCommittedAtRelease = false
        finalizationLease = nil
        faultCount = 0
    }

    public var currentText: String { textByGeneration[currentGeneration] ?? "" }
    public var inputReady: Bool { nextEpochSnapshot != nil && closePublicationCount == 1 }
    public var isRetired: Bool { closeAcknowledged }

    @discardableResult
    public mutating func applyText(
        _ text: String,
        identity: GaryxComposerInputEventIdentity,
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) -> GaryxComposerInputEventDisposition {
        guard session.payloadLifecycle.isAdmitted(by: lifecycle) else { return .rejectedToken }
        guard scopes.admitDomainEvent(from: session.scope) != .rejectedRevoked else {
            return .rejectedScope
        }
        guard identity.composerKey == session.composerKey,
              identity.sessionID == session.sessionID,
              identity.inputSessionEpoch == session.epoch else {
            faultCount += 1
            return .rejectedUnknownSession
        }
        if let finalSequence {
            if identity.inputSequence <= finalSequence {
                return .auditedTerminalDuplicate
            }
            faultCount += 1
            return .producerContractFault
        }
        guard identity.inputSequence > lastAppliedSequence else {
            return .duplicateOrOutOfOrder
        }
        guard validateReservation(identity) else {
            if let reservationID = identity.reservationID,
               terminalReservations[reservationID] != nil {
                return .auditedTerminalReservation
            }
            return .rejectedReservation
        }
        guard validateGeneration(identity.payloadGeneration) else {
            if identity.payloadGeneration < expectedEventGeneration {
                return .rejectedOldGeneration
            }
            faultCount += 1
            return .rejectedFutureGeneration
        }

        let target = GaryxComposerInputProductReducer.target(
            reservation: reservationPhase,
            producer: producerPhase
        )
        guard target != .terminalAudit else {
            return .auditedTerminalDuplicate
        }
        let generation = targetGeneration(for: target)
        textByGeneration[generation] = text
        lastAppliedSequence = identity.inputSequence
        return .applied(target: target, generation: generation)
    }

    @discardableResult
    public mutating func beginSend(
        reservationID: GaryxSendReservationID,
        followupGeneration: UInt64,
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) -> GaryxBeginSendInputDisposition {
        guard session.payloadLifecycle.isAdmitted(by: lifecycle) else { return .rejectedToken }
        guard scopes.admitDomainEvent(from: session.scope) != .rejectedRevoked else {
            return .rejectedScope
        }
        guard producerPhase == .live else { return .rejectedFinalizing }
        guard reservationPhase == .none else { return .rejectedBusy }
        precondition(followupGeneration > currentGeneration, "generation must advance")

        let envelope = currentText
        sealedEnvelope = envelope
        textByGeneration.removeValue(forKey: currentGeneration)
        textByGeneration[followupGeneration] = ""
        reservedGeneration = followupGeneration
        activeReservationID = reservationID
        reservationPhase = .sealed
        return .sealed(envelope: envelope, followupGeneration: followupGeneration)
    }

    /// Release freezes input, clears focus, and commits canonical path in one
    /// logical boundary. Async producers then drain under the lease.
    @discardableResult
    public mutating func releaseForCommittedNavigation(
        pendingProducers: Set<GaryxInputProducerKind>,
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) -> GaryxInputReleaseDisposition {
        guard session.payloadLifecycle.isAdmitted(by: lifecycle) else { return .rejectedToken }
        guard scopes.admitDomainEvent(from: session.scope) != .rejectedRevoked else {
            return .rejectedScope
        }
        guard producerPhase == .live else { return .rejectedPhase }
        producerPhase = .finalizing
        focusClearedAtRelease = true
        canonicalPathCommittedAtRelease = true
        finalizationLease = GaryxInputFinalizationLease(
            sessionID: session.sessionID,
            pendingProducers: pendingProducers
        )
        if pendingProducers.isEmpty {
            _ = producerSetReachedTerminal()
        }
        return .released
    }

    @discardableResult
    public mutating func producerReachedTerminal(
        _ producer: GaryxInputProducerKind,
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) -> GaryxProducerTerminalDisposition {
        guard session.payloadLifecycle.isAdmitted(by: lifecycle) else { return .rejectedToken }
        guard scopes.admitDomainEvent(from: session.scope) != .rejectedRevoked else {
            return .rejectedScope
        }
        guard producerPhase == .finalizing else { return .alreadyTerminal }
        finalizationLease?.producerReachedTerminal(producer)
        guard finalizationLease?.isTerminal == true else { return .stillWaiting }
        return producerSetReachedTerminal()
    }

    @discardableResult
    public mutating func cancelPendingProducers(
        _ reason: GaryxInputProducerCancellation,
        committedPath: Bool = true,
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) -> GaryxProducerTerminalDisposition {
        guard session.payloadLifecycle.isAdmitted(by: lifecycle) else { return .rejectedToken }
        guard scopes.admitDomainEvent(from: session.scope) != .rejectedRevoked else {
            return .rejectedScope
        }
        // Cancelled+visible never entered finalizing and must stay live.
        guard producerPhase == .finalizing, committedPath else { return .alreadyTerminal }
        finalizationLease?.cancelAll(reason)
        return producerSetReachedTerminal()
    }

    @discardableResult
    public mutating func commitReservation(
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) -> Bool {
        guard session.payloadLifecycle.isAdmitted(by: lifecycle),
              scopes.admitDomainEvent(from: session.scope) != .rejectedRevoked else {
            return false
        }
        guard reservationPhase == .sealed, let reservedGeneration else { return false }
        reservationPhase = .committed
        currentGeneration = reservedGeneration
        sealedEnvelope = nil
        if producerPhase == .terminal {
            performDualTerminalTransaction()
        }
        return true
    }

    @discardableResult
    public mutating func revokeReservation(
        mergeGeneration: UInt64,
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) -> Bool {
        guard session.payloadLifecycle.isAdmitted(by: lifecycle),
              scopes.admitDomainEvent(from: session.scope) != .rejectedRevoked else {
            return false
        }
        guard reservationPhase == .sealed,
              let reservedGeneration,
              mergeGeneration > reservedGeneration else {
            return false
        }
        let envelope = sealedEnvelope ?? ""
        let followup = textByGeneration[reservedGeneration] ?? ""
        textByGeneration.removeValue(forKey: reservedGeneration)
        textByGeneration[mergeGeneration] = envelope + followup
        currentGeneration = mergeGeneration
        revokedMergeGeneration = mergeGeneration
        reservationPhase = .revoked
        sealedEnvelope = nil
        if producerPhase == .terminal {
            performDualTerminalTransaction()
        }
        return true
    }

    /// Once a settled barrier has durably published its follow-up snapshot, a
    /// still-live producer can release the short-lived reservation and seal S2.
    /// The terminal record keeps S1 callbacks auditable without letting them
    /// mutate the current generation.
    @discardableResult
    public mutating func returnReservationToIdle(
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) -> Bool {
        guard session.payloadLifecycle.isAdmitted(by: lifecycle),
              scopes.admitDomainEvent(from: session.scope) != .rejectedRevoked,
              producerPhase == .live,
              let reservationID = activeReservationID else {
            return false
        }
        let outcome: GaryxReservationTerminalOutcome
        let targetGeneration: UInt64
        switch reservationPhase {
        case .committed:
            outcome = .committed
            targetGeneration = reservedGeneration ?? currentGeneration
        case .revoked:
            outcome = .revoked
            targetGeneration = revokedMergeGeneration ?? currentGeneration
        case .none, .sealed:
            return false
        }
        terminalReservations[reservationID] = GaryxInputReservationTerminalRecord(
            reservationID: reservationID,
            outcome: outcome,
            targetGeneration: targetGeneration
        )
        reservationPhase = .none
        activeReservationID = nil
        reservedGeneration = nil
        revokedMergeGeneration = nil
        sealedEnvelope = nil
        return true
    }

    public mutating func acknowledgeClose(
        lifecycle: GaryxPayloadLifecycleSnapshot,
        scopes: GaryxGatewayScopeRegistry
    ) {
        guard session.payloadLifecycle.isAdmitted(by: lifecycle),
              scopes.admitDomainEvent(from: session.scope) != .rejectedRevoked else {
            return
        }
        guard closePublicationCount == 1 else { return }
        closeAcknowledged = true
    }

    private var expectedEventGeneration: UInt64 {
        switch reservationPhase {
        case .none:
            return currentGeneration
        case .sealed, .committed, .revoked:
            return reservedGeneration ?? currentGeneration
        }
    }

    private mutating func producerSetReachedTerminal() -> GaryxProducerTerminalDisposition {
        guard producerPhase == .finalizing else { return .alreadyTerminal }
        producerPhase = .terminal
        finalSequence = lastAppliedSequence
        let bufferGeneration: UInt64
        switch reservationPhase {
        case .none:
            bufferGeneration = currentGeneration
        case .sealed, .committed:
            bufferGeneration = reservedGeneration ?? currentGeneration
        case .revoked:
            bufferGeneration = revokedMergeGeneration ?? currentGeneration
        }
        producerDrained = GaryxProducerDrainedRecord(
            sessionID: session.sessionID,
            epoch: session.epoch,
            finalSequence: lastAppliedSequence,
            bufferedText: textByGeneration[bufferGeneration] ?? ""
        )

        if reservationPhase == .sealed {
            return .producerDrainedAwaitingReservation
        }
        performDualTerminalTransaction()
        return .dualTerminalCommitted
    }

    private mutating func performDualTerminalTransaction() {
        guard producerPhase == .terminal,
              reservationPhase != .sealed,
              nextEpochSnapshot == nil else {
            return
        }
        let generation: UInt64
        switch reservationPhase {
        case .none:
            generation = currentGeneration
        case .committed:
            generation = reservedGeneration ?? currentGeneration
        case .revoked:
            generation = revokedMergeGeneration ?? currentGeneration
        case .sealed:
            return
        }
        let materialized = textByGeneration[generation] ?? ""
        finalText = materialized
        nextEpochSnapshot = GaryxComposerEpochSnapshot(
            sessionEpoch: session.epoch + 1,
            payloadGeneration: generation,
            text: materialized
        )
        closePublicationCount = 1
    }

    private func validateReservation(_ identity: GaryxComposerInputEventIdentity) -> Bool {
        switch reservationPhase {
        case .none:
            return identity.reservationID == nil
        case .sealed:
            return identity.reservationID == activeReservationID && identity.reservationID != nil
        case .committed, .revoked:
            // The activation that crossed the seal keeps its reservation tag
            // until it reaches producer terminal.
            return identity.reservationID == activeReservationID
        }
    }

    private func validateGeneration(_ generation: UInt64) -> Bool {
        generation == expectedEventGeneration
    }

    private func targetGeneration(for target: GaryxInputProductTarget) -> UInt64 {
        switch target {
        case .currentGeneration:
            return currentGeneration
        case .provisionalNextGeneration, .committedNextGeneration:
            return reservedGeneration ?? currentGeneration
        case .revokedMergeGeneration:
            return revokedMergeGeneration ?? currentGeneration
        case .terminalAudit:
            return currentGeneration
        }
    }
}

// MARK: - Composer host activation

public enum GaryxComposerHostActivationPhase: String, Codable, Sendable {
    case live
    case finalizingInput
    case closing
    case transferred
    case retained
}

public enum GaryxComposerAdapterTerminalDisposition: Equatable, Sendable {
    case none
    case sourceRemainsLive
    case destinationContinuesSameKeyAtNextEpoch
    case destinationStartsOwnKeySession
    case deferSourceUntilActive
    case deferSameKeyDestinationUntilActive
    case deferOwnKeyDestinationUntilActive
    case nextTransaction
}

/// Total policy for the activation outcome x visibility table. The state
/// machine below owns close/drain; this policy grants the next live adapter
/// only after presentation reaches terminal.
public enum GaryxComposerAdapterTerminalPolicy {
    public static func resolve(
        sourceKey: GaryxComposerKey?,
        destinationKey: GaryxComposerKey?,
        terminal: GaryxPresentationTerminalState
    ) -> GaryxComposerAdapterTerminalDisposition {
        switch (terminal.outcome, terminal.visibility) {
        case (.committed, .visible):
            guard let destinationKey else { return .none }
            return sourceKey == destinationKey
                ? .destinationContinuesSameKeyAtNextEpoch
                : .destinationStartsOwnKeySession
        case (.committed, .inactive):
            guard let destinationKey else { return .none }
            return sourceKey == destinationKey
                ? .deferSameKeyDestinationUntilActive
                : .deferOwnKeyDestinationUntilActive
        case (.committed, .superseded), (.cancelled, .superseded):
            return .nextTransaction
        case (.cancelled, .visible):
            return sourceKey == nil ? .none : .sourceRemainsLive
        case (.cancelled, .inactive):
            return sourceKey == nil ? .none : .deferSourceUntilActive
        }
    }
}

public struct GaryxComposerHostActivation: Equatable, Sendable {
    public let sourceKey: GaryxComposerKey?
    public let destinationKey: GaryxComposerKey?
    public private(set) var phase: GaryxComposerHostActivationPhase

    public init(sourceKey: GaryxComposerKey?, destinationKey: GaryxComposerKey?) {
        self.sourceKey = sourceKey
        self.destinationKey = destinationKey
        phase = sourceKey == nil ? .retained : .live
    }

    @discardableResult
    public mutating func commitReleased() -> Bool {
        guard sourceKey != nil, phase == .live else { return false }
        phase = .finalizingInput
        return true
    }

    public mutating func producerAndReservationReachedTerminal() {
        guard phase == .finalizingInput else { return }
        phase = .closing
    }

    public mutating func closeAcknowledged() {
        guard phase == .closing else { return }
        phase = destinationKey == nil ? .retained : .transferred
    }

    public mutating func cancelled() {
        guard phase == .live else { return }
        phase = .retained
    }
}

// MARK: - Scope-partitioned alias routing

public struct GaryxComposerAliasRecord: Equatable, Codable, Sendable {
    public let source: GaryxComposerKey
    public let target: GaryxComposerKey
    public fileprivate(set) var activeOrClosingSessions: Int
    public fileprivate(set) var pendingCloseAcknowledgements: Int
    public fileprivate(set) var promotionsInFlight: Int

    public init(
        source: GaryxComposerKey,
        target: GaryxComposerKey,
        activeOrClosingSessions: Int = 0,
        pendingCloseAcknowledgements: Int = 0,
        promotionsInFlight: Int = 0
    ) {
        self.source = source
        self.target = target
        self.activeOrClosingSessions = activeOrClosingSessions
        self.pendingCloseAcknowledgements = pendingCloseAcknowledgements
        self.promotionsInFlight = promotionsInFlight
    }

    public var canRetire: Bool {
        activeOrClosingSessions == 0
            && pendingCloseAcknowledgements == 0
            && promotionsInFlight == 0
    }
}

public enum GaryxComposerAliasResolution: Equatable, Sendable {
    case resolved(GaryxComposerKey)
    case rejectedRevokedScope
}

public struct GaryxComposerAliasTable: Equatable, Codable, Sendable {
    public private(set) var partitions: [GaryxGatewayScope: [GaryxComposerKey: GaryxComposerAliasRecord]]

    public init() {
        partitions = [:]
    }

    public var aliasCount: Int { partitions.values.reduce(0) { $0 + $1.count } }
    public var activeRetiringSourceCount: Int { aliasCount }
    public var estimatedBytes: Int {
        partitions.values.flatMap(\.values).reduce(0) { partial, record in
            partial + String(describing: record.source).utf8.count
                + String(describing: record.target).utf8.count + 24
        }
    }

    public mutating func establishPromotion(
        scope: GaryxGatewayScope,
        source: GaryxComposerKey,
        target: GaryxComposerKey,
        activeOrClosingSessions: Int = 0,
        pendingCloseAcknowledgements: Int = 0,
        promotionsInFlight: Int = 0
    ) {
        partitions[scope, default: [:]][source] = GaryxComposerAliasRecord(
            source: source,
            target: target,
            activeOrClosingSessions: activeOrClosingSessions,
            pendingCloseAcknowledgements: pendingCloseAcknowledgements,
            promotionsInFlight: promotionsInFlight
        )
    }

    public func resolve(
        _ key: GaryxComposerKey,
        scope: GaryxGatewayScope,
        scopes: GaryxGatewayScopeRegistry
    ) -> GaryxComposerAliasResolution {
        guard scopes.admitDomainEvent(from: scope) != .rejectedRevoked else {
            return .rejectedRevokedScope
        }
        var current = key
        var visited: Set<GaryxComposerKey> = []
        while let next = partitions[scope]?[current]?.target, visited.insert(current).inserted {
            current = next
        }
        return .resolved(current)
    }

    public mutating func markDrained(source: GaryxComposerKey, scope: GaryxGatewayScope) {
        guard var record = partitions[scope]?[source] else { return }
        record.activeOrClosingSessions = 0
        record.pendingCloseAcknowledgements = 0
        record.promotionsInFlight = 0
        partitions[scope]?[source] = record
    }

    @discardableResult
    public mutating func retireIfDrained(
        source: GaryxComposerKey,
        scope: GaryxGatewayScope
    ) -> Bool {
        guard partitions[scope]?[source]?.canRetire == true else { return false }
        partitions[scope]?.removeValue(forKey: source)
        if partitions[scope]?.isEmpty == true {
            partitions.removeValue(forKey: scope)
        }
        return true
    }
}
