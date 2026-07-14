import Foundation

/// ID-driven presentation selection for a focused Capsule. The catalog is the
/// live source of truth; the opening summary is retained only so a temporarily
/// missing or deleted row still has a stable title and source-thread fallback.
public struct GaryxCapsulePreviewSelection: Identifiable, Equatable, Sendable {
    public let id: String
    public let fallback: GaryxCapsuleSummary

    public init(id: String, fallback: GaryxCapsuleSummary) {
        self.id = id.trimmingCharacters(in: .whitespacesAndNewlines)
        self.fallback = fallback
    }

    public init(capsule: GaryxCapsuleSummary) {
        self.init(id: capsule.id, fallback: capsule)
    }
}

/// Complete identity of one focused-preview load cycle. A catalog revision
/// change, present-to-missing projection, or explicit retry cycle therefore
/// cancels the old task and starts a distinct request.
public struct GaryxCapsulePreviewLoadKey: Hashable, Equatable, Sendable {
    public let id: String
    public let projectedRevision: Int?
    public let retryGeneration: Int

    public init(id: String, projectedRevision: Int?, retryGeneration: Int) {
        self.id = id.trimmingCharacters(in: .whitespacesAndNewlines)
        self.projectedRevision = projectedRevision
        self.retryGeneration = max(0, retryGeneration)
    }
}

/// Pure catalog projection used by the focused preview and its tests.
public enum GaryxCapsulePreviewProjection {
    public static func currentSummary(
        selection: GaryxCapsulePreviewSelection,
        catalog: [GaryxCapsuleSummary]
    ) -> GaryxCapsuleSummary? {
        catalog.first { $0.id == selection.id }
    }

    public static func displaySummary(
        selection: GaryxCapsulePreviewSelection,
        catalog: [GaryxCapsuleSummary]
    ) -> GaryxCapsuleSummary {
        currentSummary(selection: selection, catalog: catalog) ?? selection.fallback
    }

    public static func loadKey(
        selection: GaryxCapsulePreviewSelection,
        catalog: [GaryxCapsuleSummary],
        retryGeneration: Int
    ) -> GaryxCapsulePreviewLoadKey {
        GaryxCapsulePreviewLoadKey(
            id: selection.id,
            projectedRevision: currentSummary(selection: selection, catalog: catalog)?.revision,
            retryGeneration: retryGeneration
        )
    }
}

public struct GaryxCapsulePreviewRenderedContent: Equatable, Sendable {
    public let html: String
    public let revision: Int?

    public init(html: String, revision: Int?) {
        self.html = html
        self.revision = revision
    }
}

public enum GaryxCapsulePreviewFailureKind: Equatable, Sendable {
    case deleted
    case retryable
    case terminal
}

/// Structured, UI-safe failure emitted by one `/serve` network attempt.
/// Cancellation is deliberately not representable here: callers must propagate
/// `CancellationError` separately and must never reduce it into a failed state.
public struct GaryxCapsulePreviewFailure: Equatable, Sendable {
    public let kind: GaryxCapsulePreviewFailureKind
    public let message: String
    public let retryAfter: TimeInterval?

    public init(
        kind: GaryxCapsulePreviewFailureKind,
        message: String,
        retryAfter: TimeInterval? = nil
    ) {
        self.kind = kind
        self.message = message
        self.retryAfter = retryAfter.map { max(0, $0) }
    }

    public var isRetryable: Bool { kind == .retryable }

    /// Reuses the Gateway client's canonical retry classifier. Returns nil for
    /// cancellation so the orchestration layer is forced to propagate it.
    public static func classify(_ error: Error) -> GaryxCapsulePreviewFailure? {
        if GaryxGatewayRetryClassifier.isCancellation(error) {
            return nil
        }
        if case let GaryxGatewayError.httpStatus(status, body, retryAfter) = error {
            if status == 404 {
                return GaryxCapsulePreviewFailure(
                    kind: .deleted,
                    message: GaryxGatewayError.httpStatus(status, body, retryAfter: retryAfter)
                        .localizedDescription
                )
            }
            let kind: GaryxCapsulePreviewFailureKind =
                GaryxGatewayRetryClassifier.isRetryableStatus(status, idempotent: true)
                ? .retryable
                : .terminal
            return GaryxCapsulePreviewFailure(
                kind: kind,
                message: GaryxGatewayError.httpStatus(status, body, retryAfter: retryAfter)
                    .localizedDescription,
                retryAfter: retryAfter
            )
        }
        let retryable = GaryxGatewayRetryClassifier.isConnectionEstablishmentError(error)
            || GaryxGatewayRetryClassifier.isAmbiguousNetworkError(error)
        return GaryxCapsulePreviewFailure(
            kind: retryable ? .retryable : .terminal,
            message: error.localizedDescription
        )
    }
}

public enum GaryxCapsulePreviewLoadPhase: Equatable, Sendable {
    case idle
    case loading
    case loaded
    case failed
    case deleted
    case paused
}

/// Request state is intentionally independent from `renderedContent`: a rev-2
/// request can be failed/retrying while rev-1 HTML remains visible.
public struct GaryxCapsulePreviewLoadStatus: Equatable, Sendable {
    public var requestedKey: GaryxCapsulePreviewLoadKey?
    public var attempt: Int
    public var phase: GaryxCapsulePreviewLoadPhase
    public var failure: GaryxCapsulePreviewFailure?
    public var retryExhausted: Bool

    public init(
        requestedKey: GaryxCapsulePreviewLoadKey? = nil,
        attempt: Int = 0,
        phase: GaryxCapsulePreviewLoadPhase = .idle,
        failure: GaryxCapsulePreviewFailure? = nil,
        retryExhausted: Bool = false
    ) {
        self.requestedKey = requestedKey
        self.attempt = max(0, attempt)
        self.phase = phase
        self.failure = failure
        self.retryExhausted = retryExhausted
    }

    public func isRetryableFailure(for key: GaryxCapsulePreviewLoadKey) -> Bool {
        requestedKey == key && phase == .failed && failure?.isRetryable == true
    }

    public func needsForegroundResume(for key: GaryxCapsulePreviewLoadKey) -> Bool {
        requestedKey == key && (phase == .paused || isRetryableFailure(for: key))
    }
}

public struct GaryxCapsulePreviewRetryPolicy: Equatable, Sendable {
    public let delays: [TimeInterval]

    public init(delays: [TimeInterval] = [2, 5, 10]) {
        self.delays = delays.map { max(0, $0) }
    }

    public var maximumNetworkAttempts: Int { delays.count + 1 }
    public static let `default` = GaryxCapsulePreviewRetryPolicy()
}

public enum GaryxCapsulePreviewRetryPhase: Equatable, Sendable {
    case idle
    case running
    case waiting
    case succeeded
    case deleted
    case terminalFailure
    case exhausted
    case cancelled
}

public struct GaryxCapsulePreviewRetryState: Equatable, Sendable {
    public var cycleGeneration: Int
    public var networkAttempt: Int
    public var phase: GaryxCapsulePreviewRetryPhase

    public init(
        cycleGeneration: Int = 0,
        networkAttempt: Int = 0,
        phase: GaryxCapsulePreviewRetryPhase = .idle
    ) {
        self.cycleGeneration = max(0, cycleGeneration)
        self.networkAttempt = max(0, networkAttempt)
        self.phase = phase
    }
}

public enum GaryxCapsulePreviewRetryEvent: Equatable, Sendable {
    case beginCycle
    case attemptStarted
    case failed(GaryxCapsulePreviewFailure)
    case retryDelayElapsed
    case succeeded
    case deleted
    case sceneInactive
    case sceneBackground
    case sceneActive
}

public enum GaryxCapsulePreviewRetryEffect: Equatable, Sendable {
    case none
    case retry(after: TimeInterval)
    case cancel
}

/// Pure bounded retry scheduler. It is the sole retry owner for focused
/// `/serve`: one initial attempt plus the three default backoff slots.
public enum GaryxCapsulePreviewRetryReducer {
    @discardableResult
    public static func reduce(
        state: inout GaryxCapsulePreviewRetryState,
        event: GaryxCapsulePreviewRetryEvent,
        policy: GaryxCapsulePreviewRetryPolicy = .default
    ) -> GaryxCapsulePreviewRetryEffect {
        switch event {
        case .beginCycle, .sceneActive:
            state.cycleGeneration &+= 1
            state.networkAttempt = 0
            state.phase = .running
            return .none
        case .attemptStarted:
            guard state.phase != .cancelled else { return .none }
            state.networkAttempt &+= 1
            state.phase = .running
            return .none
        case let .failed(failure):
            switch failure.kind {
            case .deleted:
                state.phase = .deleted
                return .none
            case .terminal:
                state.phase = .terminalFailure
                return .none
            case .retryable:
                let delayIndex = state.networkAttempt - 1
                guard delayIndex >= 0, delayIndex < policy.delays.count else {
                    state.phase = .exhausted
                    return .none
                }
                state.phase = .waiting
                return .retry(after: max(policy.delays[delayIndex], failure.retryAfter ?? 0))
            }
        case .retryDelayElapsed:
            guard state.phase == .waiting else { return .none }
            state.phase = .running
            return .none
        case .succeeded:
            state.phase = .succeeded
            return .none
        case .deleted:
            state.phase = .deleted
            return .none
        case .sceneInactive, .sceneBackground:
            state.phase = .cancelled
            return .cancel
        }
    }
}
