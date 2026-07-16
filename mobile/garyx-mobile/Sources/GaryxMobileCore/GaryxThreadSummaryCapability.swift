import Foundation

public enum GaryxThreadSummaryCapabilityState: Equatable, Sendable {
    case unknown
    case supported
    case unsupported
}

public enum GaryxThreadSummaryCapabilityProbeResult: Equatable, Sendable {
    case httpStatus(Int)
    case failed
}

public struct GaryxThreadSummaryCapabilityResolution: Equatable, Sendable {
    public var state: GaryxThreadSummaryCapabilityState
    public var runtimeEpoch: UInt64
    public var capabilityGeneration: UInt64
    /// True for exactly one waiter when a probe commits a transition to
    /// supported. That waiter owns the single enhanced favorites replacement.
    public var becameSupported: Bool
    public var probeFailed: Bool

    public init(
        state: GaryxThreadSummaryCapabilityState,
        runtimeEpoch: UInt64,
        capabilityGeneration: UInt64,
        becameSupported: Bool,
        probeFailed: Bool
    ) {
        self.state = state
        self.runtimeEpoch = runtimeEpoch
        self.capabilityGeneration = capabilityGeneration
        self.becameSupported = becameSupported
        self.probeFailed = probeFailed
    }
}

/// Runtime-epoch isolated, single-flight feature probe. The owner retains the
/// unstructured probe task; waiter cancellation therefore cannot cancel it.
public actor GaryxThreadSummaryCapabilityStateMachine {
    public typealias Probe = @Sendable () async -> GaryxThreadSummaryCapabilityProbeResult

    private struct InFlight {
        var id: UInt64
        var runtimeEpoch: UInt64
        var task: Task<GaryxThreadSummaryCapabilityProbeResult, Never>
    }

    public private(set) var state: GaryxThreadSummaryCapabilityState
    public private(set) var runtimeEpoch: UInt64
    public private(set) var capabilityGeneration: UInt64

    private let probe: Probe
    private var inFlight: InFlight?
    private var nextProbeId: UInt64 = 1

    public init(
        runtimeEpoch: UInt64 = 0,
        probe: @escaping Probe
    ) {
        self.runtimeEpoch = runtimeEpoch
        self.probe = probe
        state = .unknown
        capabilityGeneration = 0
    }

    public func currentResolution() -> GaryxThreadSummaryCapabilityResolution {
        resolution(becameSupported: false, probeFailed: false)
    }

    public func resolve() async -> GaryxThreadSummaryCapabilityResolution {
        guard state == .unknown else {
            return resolution(becameSupported: false, probeFailed: false)
        }
        let flight: InFlight
        if let inFlight {
            flight = inFlight
        } else {
            let id = nextProbeId
            nextProbeId &+= 1
            let task = Task { await probe() }
            flight = InFlight(id: id, runtimeEpoch: runtimeEpoch, task: task)
            inFlight = flight
        }
        let result = await flight.task.value
        // A cancelled consumer must not claim the one-shot supported
        // transition: it may never perform the required enhanced favorites
        // replacement. The owner-held flight remains available for another
        // waiter (or a later consumer) to commit.
        guard !Task.isCancelled else {
            return resolution(becameSupported: false, probeFailed: false)
        }
        return finish(flight: flight, result: result)
    }

    /// Gateway reset/reconnect establishes a new isolation epoch. An old
    /// result is structurally unable to commit into this domain.
    public func reset(runtimeEpoch: UInt64) {
        inFlight?.task.cancel()
        inFlight = nil
        self.runtimeEpoch = runtimeEpoch
        capabilityGeneration &+= 1
        state = .unknown
        nextProbeId &+= 1
    }

    private func finish(
        flight: InFlight,
        result: GaryxThreadSummaryCapabilityProbeResult
    ) -> GaryxThreadSummaryCapabilityResolution {
        guard let current = inFlight,
              current.id == flight.id,
              current.runtimeEpoch == flight.runtimeEpoch,
              flight.runtimeEpoch == runtimeEpoch else {
            return resolution(becameSupported: false, probeFailed: false)
        }
        inFlight = nil
        switch result {
        case .httpStatus(200):
            let transitioned = state != .supported
            state = .supported
            if transitioned { capabilityGeneration &+= 1 }
            return resolution(becameSupported: transitioned, probeFailed: false)
        case .httpStatus(404):
            state = .unsupported
            return resolution(becameSupported: false, probeFailed: false)
        case .httpStatus, .failed:
            // Auth, server, network, and decode failures are ordinary errors;
            // they never become a sticky old-gateway classification.
            state = .unknown
            return resolution(becameSupported: false, probeFailed: true)
        }
    }

    private func resolution(
        becameSupported: Bool,
        probeFailed: Bool
    ) -> GaryxThreadSummaryCapabilityResolution {
        GaryxThreadSummaryCapabilityResolution(
            state: state,
            runtimeEpoch: runtimeEpoch,
            capabilityGeneration: capabilityGeneration,
            becameSupported: becameSupported,
            probeFailed: probeFailed
        )
    }
}
