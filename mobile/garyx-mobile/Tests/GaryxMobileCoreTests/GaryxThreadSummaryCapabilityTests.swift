import XCTest
@testable import GaryxMobileCore

final class GaryxThreadSummaryCapabilityTests: XCTestCase {
    func testColdStart200TransitionsSupportedAndOnlyOneResolutionOwnsReplacement() async {
        let probe = CapabilityProbeGate()
        let machine = GaryxThreadSummaryCapabilityStateMachine(runtimeEpoch: 8) {
            await probe.run()
        }
        let waiters = (0..<12).map { _ in
            Task { await machine.resolve() }
        }
        let probeStarted = await waitForProbeCalls(1, probe: probe)
        XCTAssertTrue(probeStarted)
        await probe.resumeNext(.httpStatus(200))

        var resolutions: [GaryxThreadSummaryCapabilityResolution] = []
        for waiter in waiters {
            resolutions.append(await waiter.value)
        }
        let initialProbeCount = await probe.callCount
        XCTAssertEqual(initialProbeCount, 1)
        XCTAssertTrue(resolutions.allSatisfy { $0.state == .supported && $0.runtimeEpoch == 8 })
        XCTAssertEqual(resolutions.filter(\.becameSupported).count, 1)
        XCTAssertEqual(Set(resolutions.map(\.capabilityGeneration)), [1])

        let stable = await machine.resolve()
        XCTAssertEqual(stable.state, .supported)
        XCTAssertFalse(stable.becameSupported)
        let stableProbeCount = await probe.callCount
        XCTAssertEqual(stableProbeCount, 1)
    }

    func testColdStartExact404IsOnlyUnsupportedClassification() async {
        let machine = GaryxThreadSummaryCapabilityStateMachine(runtimeEpoch: 2) {
            .httpStatus(404)
        }
        let resolution = await machine.resolve()
        XCTAssertEqual(resolution.state, .unsupported)
        XCTAssertFalse(resolution.probeFailed)
        XCTAssertFalse(resolution.becameSupported)
        XCTAssertEqual(resolution.capabilityGeneration, 0)
    }

    func testAuthServerNetworkAndDecodeFailuresRemainUnknownAndRetry() async {
        for failure in [
            GaryxThreadSummaryCapabilityProbeResult.httpStatus(401),
            .httpStatus(403),
            .httpStatus(500),
            .httpStatus(400),
            .failed,
        ] {
            let sequence = CapabilityProbeSequence([failure, .httpStatus(200)])
            let machine = GaryxThreadSummaryCapabilityStateMachine(runtimeEpoch: 4) {
                await sequence.next()
            }
            let failed = await machine.resolve()
            XCTAssertEqual(failed.state, .unknown, "failure: \(failure)")
            XCTAssertTrue(failed.probeFailed, "failure: \(failure)")

            let recovered = await machine.resolve()
            XCTAssertEqual(recovered.state, .supported)
            XCTAssertTrue(recovered.becameSupported)
            let callCount = await sequence.callCount
            XCTAssertEqual(callCount, 2)
        }
    }

    func testCancelledWaiterDoesNotCancelProbeOrClaimSupportedTransition() async {
        let probe = CapabilityProbeGate()
        let machine = GaryxThreadSummaryCapabilityStateMachine(runtimeEpoch: 6) {
            await probe.run()
        }
        let cancelled = Task { await machine.resolve() }
        let probeStarted = await waitForProbeCalls(1, probe: probe)
        XCTAssertTrue(probeStarted)
        let survivor = Task { await machine.resolve() }
        cancelled.cancel()
        await probe.resumeNext(.httpStatus(200))

        let cancelledResolution = await cancelled.value
        let survivorResolution = await survivor.value
        XCTAssertFalse(cancelledResolution.becameSupported)
        XCTAssertEqual(survivorResolution.state, .supported)
        XCTAssertTrue(survivorResolution.becameSupported)
        let callCount = await probe.callCount
        XCTAssertEqual(callCount, 1)
    }

    func testRuntimeEpochResetFencesOldProbeAndReconnectForcesFreshSupportedBarrier() async {
        let probe = CapabilityProbeGate()
        let machine = GaryxThreadSummaryCapabilityStateMachine(runtimeEpoch: 10) {
            await probe.run()
        }
        let old = Task { await machine.resolve() }
        let oldProbeStarted = await waitForProbeCalls(1, probe: probe)
        XCTAssertTrue(oldProbeStarted)

        await machine.reset(runtimeEpoch: 11)
        await probe.resumeNext(.httpStatus(200))
        let stale = await old.value
        XCTAssertEqual(stale.runtimeEpoch, 11)
        XCTAssertEqual(stale.state, .unknown)
        XCTAssertFalse(stale.becameSupported)

        let replacement = Task { await machine.resolve() }
        let replacementProbeStarted = await waitForProbeCalls(2, probe: probe)
        XCTAssertTrue(replacementProbeStarted)
        await probe.resumeNext(.httpStatus(200))
        let current = await replacement.value
        XCTAssertEqual(current.runtimeEpoch, 11)
        XCTAssertEqual(current.state, .supported)
        XCTAssertTrue(current.becameSupported)
        XCTAssertEqual(current.capabilityGeneration, 2)
    }

    func testUnsupportedReconnectResetsToUnknownThenSupportedForcesReplacement() async {
        let sequence = CapabilityProbeSequence([.httpStatus(404), .httpStatus(200)])
        let machine = GaryxThreadSummaryCapabilityStateMachine(runtimeEpoch: 1) {
            await sequence.next()
        }
        let unsupported = await machine.resolve()
        XCTAssertEqual(unsupported.state, .unsupported)
        await machine.reset(runtimeEpoch: 2)
        let afterReset = await machine.currentResolution()
        XCTAssertEqual(afterReset.state, .unknown)
        XCTAssertEqual(afterReset.runtimeEpoch, 2)

        let upgraded = await machine.resolve()
        XCTAssertEqual(upgraded.state, .supported)
        XCTAssertTrue(upgraded.becameSupported)
        let callCount = await sequence.callCount
        XCTAssertEqual(callCount, 2)
    }

    private func waitForProbeCalls(
        _ expected: Int,
        probe: CapabilityProbeGate
    ) async -> Bool {
        for _ in 0..<10_000 {
            if await probe.callCount >= expected { return true }
            await Task.yield()
        }
        return false
    }
}

private actor CapabilityProbeGate {
    private(set) var callCount = 0
    private var continuations: [CheckedContinuation<GaryxThreadSummaryCapabilityProbeResult, Never>] = []

    func run() async -> GaryxThreadSummaryCapabilityProbeResult {
        callCount += 1
        return await withCheckedContinuation { continuation in
            continuations.append(continuation)
        }
    }

    func resumeNext(_ result: GaryxThreadSummaryCapabilityProbeResult) {
        guard !continuations.isEmpty else { return }
        continuations.removeFirst().resume(returning: result)
    }
}

private actor CapabilityProbeSequence {
    private var results: [GaryxThreadSummaryCapabilityProbeResult]
    private(set) var callCount = 0

    init(_ results: [GaryxThreadSummaryCapabilityProbeResult]) {
        self.results = results
    }

    func next() -> GaryxThreadSummaryCapabilityProbeResult {
        callCount += 1
        return results.removeFirst()
    }
}
