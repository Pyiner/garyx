import Foundation

// MARK: - Gateway scope lifecycle

public struct GaryxGatewayScope: Hashable, Codable, Sendable {
    public let identity: String
    public let epoch: UInt64

    public init(identity: String, epoch: UInt64) {
        precondition(!identity.isEmpty, "gateway scope identity must not be empty")
        self.identity = identity
        self.epoch = epoch
    }
}

/// UI/network work captures both its durable gateway partition and the exact
/// activation that launched it. A suspended partition may later become active
/// again so scope equality alone cannot reject a completion from before the
/// switch-away. The activation sequence supplies that ephemeral CAS while the
/// scope keeps durable composer data recoverable when the user switches back.
public struct GaryxGatewayRequestToken: Hashable, Codable, Sendable {
    public let scope: GaryxGatewayScope
    public let activationSequence: UInt64

    public init(scope: GaryxGatewayScope, activationSequence: UInt64) {
        precondition(activationSequence > 0, "gateway activation sequence must be positive")
        self.scope = scope
        self.activationSequence = activationSequence
    }
}

public enum GaryxGatewayScopeLifecycle: String, Codable, Sendable {
    case active
    case suspended
    case revoked
}

public enum GaryxGatewayScopeEventAdmission: Equatable, Sendable {
    case acceptedActive
    case acceptedSuspendedPartition
    case rejectedRevoked
}

/// Bounded scope registry. Revoked epochs collapse into one monotonic watermark
/// per gateway identity rather than accumulating epoch tombstones.
public struct GaryxGatewayScopeRegistry: Equatable, Sendable {
    public private(set) var lifecycles: [GaryxGatewayScope: GaryxGatewayScopeLifecycle]
    public private(set) var activeScope: GaryxGatewayScope?
    public private(set) var revokedThroughEpoch: [String: UInt64]

    public init(
        initialActiveScope: GaryxGatewayScope? = nil,
        revokedThroughEpoch: [String: UInt64] = [:]
    ) {
        self.revokedThroughEpoch = revokedThroughEpoch
        if let initialActiveScope,
           initialActiveScope.epoch > (revokedThroughEpoch[initialActiveScope.identity] ?? 0) {
            activeScope = initialActiveScope
            lifecycles = [initialActiveScope: .active]
        } else {
            activeScope = nil
            lifecycles = [:]
        }
    }

    public var authenticationRequired: Bool { activeScope == nil }

    @discardableResult
    public mutating func switchActive(to scope: GaryxGatewayScope) -> Bool {
        guard scope.epoch > (revokedThroughEpoch[scope.identity] ?? 0) else {
            return false
        }
        if let old = activeScope, old != scope {
            lifecycles[old] = .suspended
        }
        activeScope = scope
        lifecycles[scope] = .active
        return true
    }

    @discardableResult
    public mutating func suspendActive() -> GaryxGatewayScope? {
        guard let activeScope else { return nil }
        lifecycles[activeScope] = .suspended
        self.activeScope = nil
        return activeScope
    }

    @discardableResult
    public mutating func revoke(_ scope: GaryxGatewayScope) -> Bool {
        guard lifecycles[scope] != nil else { return false }
        let watermark = revokedThroughEpoch[scope.identity] ?? 0
        revokedThroughEpoch[scope.identity] = max(watermark, scope.epoch)
        // The watermark is the complete rejection proof; retaining one
        // tombstone per revoked epoch would make logout churn unbounded.
        lifecycles.removeValue(forKey: scope)
        if activeScope == scope {
            activeScope = nil
        }
        return scope.epoch > watermark
    }

    public func lifecycle(of scope: GaryxGatewayScope) -> GaryxGatewayScopeLifecycle {
        if scope.epoch <= (revokedThroughEpoch[scope.identity] ?? 0) {
            return .revoked
        }
        return lifecycles[scope] ?? .revoked
    }

    public func admitDomainEvent(from scope: GaryxGatewayScope) -> GaryxGatewayScopeEventAdmission {
        switch lifecycle(of: scope) {
        case .active:
            .acceptedActive
        case .suspended:
            .acceptedSuspendedPartition
        case .revoked:
            .rejectedRevoked
        }
    }
}

// MARK: - Typed preparation

public enum GaryxPrepareOutcome<Prepared: Equatable & Sendable>: Equatable, Sendable {
    case ready(Prepared)
    case userVisibleNotFound
    case retryableFailure(message: String)
    case authenticationRequired
    case cancelledOrStale
    case internalFault(code: String)

    public func map<Mapped: Equatable & Sendable>(
        _ transform: (Prepared) -> Mapped
    ) -> GaryxPrepareOutcome<Mapped> {
        switch self {
        case .ready(let value):
            .ready(transform(value))
        case .userVisibleNotFound:
            .userVisibleNotFound
        case .retryableFailure(let message):
            .retryableFailure(message: message)
        case .authenticationRequired:
            .authenticationRequired
        case .cancelledOrStale:
            .cancelledOrStale
        case .internalFault(let code):
            .internalFault(code: code)
        }
    }

    public var preparedValue: Prepared? {
        guard case .ready(let value) = self else { return nil }
        return value
    }
}

public protocol GaryxNavigationIntentPreparing {
    associatedtype Intent: Sendable
    associatedtype Prepared: Equatable & Sendable

    /// Preparation may read resolvers/stores but must not mutate route state.
    func prepare(_ intent: Intent) async -> GaryxPrepareOutcome<Prepared>
}

public struct GaryxNavigationIntentID: RawRepresentable, Hashable, Codable, Sendable {
    public let rawValue: String

    public init(rawValue: String) {
        precondition(!rawValue.isEmpty, "navigation intent ID must not be empty")
        self.rawValue = rawValue
    }
}

public enum GaryxNavigationIntentEffect: Equatable, Sendable {
    case logout(scope: GaryxGatewayScope)
    case routeInvalidation(fallback: GaryxRouteDestination?)
    case gatewayScopeChange(GaryxGatewayScope)
    case ordinaryNavigation(GaryxRouteDestination)
}

public enum GaryxNavigationIntentPriority: Int, Comparable, Codable, Sendable {
    case ordinaryNavigation = 0
    case gatewayScopeChange = 1
    case safetyForced = 2

    public static func < (lhs: Self, rhs: Self) -> Bool { lhs.rawValue < rhs.rawValue }
}

public enum GaryxNavigationIntentCoalescingKey: Hashable, Codable, Sendable {
    case logout
    case routeInvalidation
    case gatewayScopeChange
    case ordinaryNavigation
}

public enum GaryxRelativeDependencyMismatchPolicy: Equatable, Sendable {
    case reprepare
    case discard
}

public enum GaryxNavigationIntentDependency: Equatable, Sendable {
    case absolute
    case relative(
        base: GaryxRouteInstanceID,
        payloadRevision: UInt64,
        stackRevision: UInt64,
        mismatch: GaryxRelativeDependencyMismatchPolicy
    )
}

public struct GaryxPreparedNavigationIntent: Equatable, Sendable {
    public let id: GaryxNavigationIntentID
    public let effect: GaryxNavigationIntentEffect
    public let dependency: GaryxNavigationIntentDependency

    public init(
        id: GaryxNavigationIntentID,
        effect: GaryxNavigationIntentEffect,
        dependency: GaryxNavigationIntentDependency
    ) {
        self.id = id
        self.effect = effect
        self.dependency = dependency
    }

    public var priority: GaryxNavigationIntentPriority {
        switch effect {
        case .logout, .routeInvalidation:
            .safetyForced
        case .gatewayScopeChange:
            .gatewayScopeChange
        case .ordinaryNavigation:
            .ordinaryNavigation
        }
    }

    public var coalescingKey: GaryxNavigationIntentCoalescingKey {
        switch effect {
        case .logout:
            .logout
        case .routeInvalidation:
            .routeInvalidation
        case .gatewayScopeChange:
            .gatewayScopeChange
        case .ordinaryNavigation:
            .ordinaryNavigation
        }
    }
}

/// The three values checked atomically when an asynchronous preparation
/// completes. This prevents a resolver that ignores cancellation from reviving
/// an older intent.
public struct GaryxNavigationIntentEpoch: Equatable, Sendable {
    public let scope: GaryxGatewayScope
    public let intentEpoch: UInt64
    public let coalescingEpoch: UInt64

    public init(scope: GaryxGatewayScope, intentEpoch: UInt64, coalescingEpoch: UInt64) {
        self.scope = scope
        self.intentEpoch = intentEpoch
        self.coalescingEpoch = coalescingEpoch
    }
}

public struct GaryxNavigationPreparationTicket: Equatable, Sendable {
    public let intentID: GaryxNavigationIntentID
    public let coalescingKey: GaryxNavigationIntentCoalescingKey
    public let epoch: GaryxNavigationIntentEpoch
}

public enum GaryxNavigationQueueResult: Equatable, Sendable {
    case queued
    case admittedImmediately
    case presentationDismissalRequired
    case authenticationRequired
    case userVisibleNotFound
    case retryableFailure(message: String)
    case cancelledOrStale
    case internalFault(code: String)
    case stalePreparation
    case reprepareRequired
    case dependencyDiscarded
}

public enum GaryxNavigationTransactionStatus: Equatable, Sendable {
    case terminal
    case nonTerminal
}

public enum GaryxNavigationAdmissionAction: Equatable, Sendable {
    case none
    case waitForTransactionTerminal
    case waitForPresentationBarrier
    case requestPresentationDismissal
    case admit
}

public enum GaryxNavigationDependencyDisposition: Equatable, Sendable {
    case admit
    case reprepare
    case discard
}

public struct GaryxNavigationIntentCoordinator: Equatable, Sendable {
    public private(set) var transactionStatus: GaryxNavigationTransactionStatus
    public private(set) var authenticationBarrier: Bool
    public private(set) var authenticationBarrierOriginScope: GaryxGatewayScope?
    public private(set) var queued: [GaryxPreparedNavigationIntent]

    private var nextIntentEpoch: UInt64
    private var latestIntentEpoch: [GaryxNavigationIntentID: UInt64]
    private var coalescingEpochs: [GaryxNavigationIntentCoalescingKey: UInt64]

    public init(
        transactionStatus: GaryxNavigationTransactionStatus = .terminal,
        authenticationBarrier: Bool = false
    ) {
        self.transactionStatus = transactionStatus
        self.authenticationBarrier = authenticationBarrier
        authenticationBarrierOriginScope = nil
        queued = []
        nextIntentEpoch = 0
        latestIntentEpoch = [:]
        coalescingEpochs = [:]
    }

    public mutating func setTransactionStatus(_ status: GaryxNavigationTransactionStatus) {
        transactionStatus = status
    }

    /// Starts read-only preparation and returns the epoch triple that completion
    /// must still own.
    public mutating func beginPreparation(
        intentID: GaryxNavigationIntentID,
        key: GaryxNavigationIntentCoalescingKey,
        scope: GaryxGatewayScope
    ) -> GaryxNavigationPreparationTicket {
        nextIntentEpoch &+= 1
        let laneEpoch = (coalescingEpochs[key] ?? 0) &+ 1
        coalescingEpochs[key] = laneEpoch
        latestIntentEpoch[intentID] = nextIntentEpoch
        return GaryxNavigationPreparationTicket(
            intentID: intentID,
            coalescingKey: key,
            epoch: GaryxNavigationIntentEpoch(
                scope: scope,
                intentEpoch: nextIntentEpoch,
                coalescingEpoch: laneEpoch
            )
        )
    }

    public func owns(
        _ ticket: GaryxNavigationPreparationTicket,
        scopes: GaryxGatewayScopeRegistry
    ) -> Bool {
        scopes.activeScope == ticket.epoch.scope
            && latestIntentEpoch[ticket.intentID] == ticket.epoch.intentEpoch
            && coalescingEpochs[ticket.coalescingKey] == ticket.epoch.coalescingEpoch
    }

    /// Completes preparation. Non-ready outcomes are typed and never enter the
    /// queue. Ready intents are admitted only when no transaction is active.
    @discardableResult
    public mutating func completePreparation(
        _ ticket: GaryxNavigationPreparationTicket,
        outcome: GaryxPrepareOutcome<GaryxPreparedNavigationIntent>,
        scopes: GaryxGatewayScopeRegistry,
        routeState: GaryxCanonicalRouteState,
        presentationBarrier: Bool
    ) -> GaryxNavigationQueueResult {
        guard owns(ticket, scopes: scopes) else { return .stalePreparation }

        switch outcome {
        case .authenticationRequired:
            return .authenticationRequired
        case .userVisibleNotFound:
            return .userVisibleNotFound
        case .retryableFailure(let message):
            return .retryableFailure(message: message)
        case .cancelledOrStale:
            return .cancelledOrStale
        case .internalFault(let code):
            return .internalFault(code: code)
        case .ready(let intent):
            guard intent.id == ticket.intentID,
                  intent.coalescingKey == ticket.coalescingKey else {
                return .stalePreparation
            }
            if authenticationBarrier,
               intent.priority == .ordinaryNavigation {
                return .authenticationRequired
            }
            switch validateDependency(intent.dependency, routeState: routeState) {
            case .valid:
                guard enqueue(intent) else { return .cancelledOrStale }
            case .reprepare:
                return .reprepareRequired
            case .discard:
                return .dependencyDiscarded
            }
            switch nextAdmissionAction(presentationBarrier: presentationBarrier) {
            case .admit:
                return .admittedImmediately
            case .requestPresentationDismissal:
                return .presentationDismissalRequired
            case .none, .waitForTransactionTerminal, .waitForPresentationBarrier:
                return .queued
            }
        }
    }

    /// Composes the transaction boundary with the PresentationLease barrier.
    /// Ordinary and gateway-change intents wait without unloading the presenter;
    /// a safety-forced intent asks the host to dismiss the lease tree, then waits
    /// for the exactly-once release event before it can be drained atomically.
    public func nextAdmissionAction(
        presentationBarrier: Bool
    ) -> GaryxNavigationAdmissionAction {
        guard !queued.isEmpty else { return .none }
        guard transactionStatus == .terminal else { return .waitForTransactionTerminal }
        guard presentationBarrier else { return .admit }
        return queued.contains(where: { $0.priority == .safetyForced })
            ? .requestPresentationDismissal
            : .waitForPresentationBarrier
    }

    /// Returns the next effects only at a terminal boundary. Safety effects use
    /// distinct keys and therefore survive together; their canonical order is
    /// route invalidation then logout, making both arrival orders equivalent.
    public mutating func drainAdmissible(
        presentationBarrier: Bool
    ) -> [GaryxPreparedNavigationIntent] {
        guard nextAdmissionAction(presentationBarrier: presentationBarrier) == .admit else {
            return []
        }
        let ordered = queued.sorted { lhs, rhs in
            if lhs.priority != rhs.priority { return lhs.priority > rhs.priority }
            return canonicalOrder(lhs.coalescingKey) < canonicalOrder(rhs.coalescingKey)
        }
        queued.removeAll()
        if ordered.contains(where: {
            if case .logout = $0.effect { return true }
            return false
        }) {
            authenticationBarrier = true
            authenticationBarrierOriginScope = ordered.compactMap { intent in
                if case .logout(let scope) = intent.effect { return scope }
                return nil
            }.last
        }
        return ordered
    }

    @discardableResult
    public mutating func authenticated(
        in scope: GaryxGatewayScope,
        scopes: GaryxGatewayScopeRegistry
    ) -> Bool {
        guard authenticationBarrier,
              scopes.activeScope == scope,
              scopes.lifecycle(of: scope) == .active else {
            return false
        }
        if let origin = authenticationBarrierOriginScope {
            guard scope.identity != origin.identity || scope.epoch > origin.epoch else {
                return false
            }
        }
        authenticationBarrier = false
        authenticationBarrierOriginScope = nil
        // Ordinary intents rejected behind the barrier are intentionally not
        // retained or automatically reprepared.
        return true
    }

    /// Revalidates a prepared dependency at the actual admission boundary.
    /// A relative intent can wait behind a gesture or modal long after its
    /// resolver completed; the canonical stack may have changed meanwhile.
    public func dependencyDisposition(
        for intent: GaryxPreparedNavigationIntent,
        routeState: GaryxCanonicalRouteState
    ) -> GaryxNavigationDependencyDisposition {
        switch validateDependency(intent.dependency, routeState: routeState) {
        case .valid:
            .admit
        case .reprepare:
            .reprepare
        case .discard:
            .discard
        }
    }

    private enum DependencyValidation {
        case valid
        case reprepare
        case discard
    }

    private func validateDependency(
        _ dependency: GaryxNavigationIntentDependency,
        routeState: GaryxCanonicalRouteState
    ) -> DependencyValidation {
        switch dependency {
        case .absolute:
            return .valid
        case .relative(let base, let payloadRevision, let stackRevision, let mismatch):
            let matches = routeState.stackRevision == stackRevision
                && routeState.path.last?.id == base
                && routeState.path.last?.payloadRevision == payloadRevision
            guard !matches else { return .valid }
            return mismatch == .reprepare ? .reprepare : .discard
        }
    }

    private mutating func enqueue(_ intent: GaryxPreparedNavigationIntent) -> Bool {
        switch intent.priority {
        case .safetyForced:
            queued.removeAll(where: { $0.priority < .safetyForced })
            replaceSameKey(with: intent)
            return true
        case .gatewayScopeChange:
            guard !queued.contains(where: { $0.priority == .safetyForced }) else { return false }
            queued.removeAll(where: { $0.priority == .ordinaryNavigation })
            replaceSameKey(with: intent)
            return true
        case .ordinaryNavigation:
            guard !queued.contains(where: { $0.priority > .ordinaryNavigation }) else {
                return false
            }
            replaceSameKey(with: intent)
            return true
        }
    }

    private mutating func replaceSameKey(with intent: GaryxPreparedNavigationIntent) {
        queued.removeAll(where: { $0.coalescingKey == intent.coalescingKey })
        queued.append(intent)
    }

    private func canonicalOrder(_ key: GaryxNavigationIntentCoalescingKey) -> Int {
        switch key {
        case .routeInvalidation: 0
        case .logout: 1
        case .gatewayScopeChange: 2
        case .ordinaryNavigation: 3
        }
    }
}
