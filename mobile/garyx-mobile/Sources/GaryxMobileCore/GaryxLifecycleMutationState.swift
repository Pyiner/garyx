import Foundation

public enum GaryxLifecycleMutationKind: String, Equatable, Sendable {
    case archive
    case delete
}

public enum GaryxLifecycleMutationPolicy {
    public static let joinWindowSeconds: TimeInterval = 6
    public static let transportTimeoutSeconds: TimeInterval = 8
    public static let retryDelaysNanoseconds: [UInt64] = [
        1_000_000_000,
        2_000_000_000,
        4_000_000_000,
        8_000_000_000,
        8_000_000_000,
    ]
}

public struct GaryxLifecycleMutationRequest: Equatable, Sendable {
    public var kind: GaryxLifecycleMutationKind
    public var threadId: String
    public var endpointKeys: [String]
    public var operationId: String
    public var expectedStoreIncarnation: String
    public var gatewayScope: String
    public var runtimeGeneration: UUID

    public init(
        kind: GaryxLifecycleMutationKind,
        threadId: String,
        endpointKeys: [String] = [],
        operationId: UUID = UUID(),
        expectedStoreIncarnation: String,
        gatewayScope: String,
        runtimeGeneration: UUID
    ) {
        self.kind = kind
        self.threadId = threadId
        self.endpointKeys = endpointKeys
        self.operationId = operationId.uuidString.lowercased()
        self.expectedStoreIncarnation = expectedStoreIncarnation
        self.gatewayScope = gatewayScope
        self.runtimeGeneration = runtimeGeneration
    }
}

public struct GaryxLifecycleMutationAttempt: Equatable, Sendable {
    public var request: GaryxLifecycleMutationRequest
    public var attemptNumber: Int
}

public enum GaryxLifecycleMutationDecision<Response: Sendable>: Sendable {
    case applied(Response)
    case rejected(code: String, message: String)
    case operationIdConflict(message: String)
    case retry(delayNanoseconds: UInt64)
    case exhausted(message: String)
}

extension GaryxLifecycleMutationDecision: Equatable where Response: Equatable {}

public struct GaryxLifecycleMutationState: Equatable, Sendable {
    private enum Phase: Equatable, Sendable {
        case ready
        case awaitingResult
        case waitingRetry
        case terminal
    }

    public let request: GaryxLifecycleMutationRequest
    public private(set) var attemptCount: Int
    private var phase: Phase

    public init(request: GaryxLifecycleMutationRequest) {
        self.request = request
        attemptCount = 0
        phase = .ready
    }

    /// Returns the next immutable request ticket. Every returned ticket is
    /// dispatched exactly once; operation identity never changes across them.
    public mutating func nextAttempt() -> GaryxLifecycleMutationAttempt? {
        guard phase == .ready || phase == .waitingRetry else { return nil }
        attemptCount += 1
        phase = .awaitingResult
        return GaryxLifecycleMutationAttempt(
            request: request,
            attemptNumber: attemptCount
        )
    }

    public mutating func settle<Response: Sendable>(
        _ result: GaryxGatewayMutationResult<Response>
    ) -> GaryxLifecycleMutationDecision<Response> {
        precondition(phase == .awaitingResult, "lifecycle result without an owned attempt")

        switch result {
        case .ok(let response):
            phase = .terminal
            return .applied(response)
        case .definitiveEndpointResponse(let response):
            let code = response.error.code
            let message = response.error.message ?? code
            if code == "operation_in_progress" || code == "unavailable" {
                return scheduleRetry(message: message)
            }
            phase = .terminal
            if code == "operation_id_conflict" {
                return .operationIdConflict(message: message)
            }
            return .rejected(code: code, message: message)
        case .ambiguous(let response):
            return scheduleRetry(message: response.message)
        case .notSent(let message):
            return scheduleRetry(message: message)
        }
    }

    private mutating func scheduleRetry<Response: Sendable>(
        message: String
    ) -> GaryxLifecycleMutationDecision<Response> {
        let retryIndex = attemptCount - 1
        guard GaryxLifecycleMutationPolicy.retryDelaysNanoseconds.indices.contains(retryIndex) else {
            phase = .terminal
            return .exhausted(
                message: message.isEmpty
                    ? "Thread lifecycle result was unavailable."
                    : message
            )
        }
        phase = .waitingRetry
        return .retry(
            delayNanoseconds: GaryxLifecycleMutationPolicy.retryDelaysNanoseconds[retryIndex]
        )
    }
}
