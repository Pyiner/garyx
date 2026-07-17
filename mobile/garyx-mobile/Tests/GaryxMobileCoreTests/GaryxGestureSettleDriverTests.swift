import XCTest
@testable import GaryxMobileCore

@MainActor
final class GaryxGestureSettleDriverTests: XCTestCase {
    func testSettleStartsAtReleaseValueAndVelocity() throws {
        let harness = Harness()
        var samples: [GaryxMotionPhysics.MotionSample] = []
        harness.driver.settle(
            from: 72,
            to: 0,
            initialVelocity: 640,
            curve: .init(response: 0.22, dampingRatio: 0.88),
            onUpdate: { samples.append($0) }
        )

        let seam = try XCTUnwrap(samples.first)
        XCTAssertEqual(seam.value, 72, accuracy: 1e-12)
        XCTAssertEqual(seam.velocity, 640, accuracy: 1e-12)
        XCTAssertTrue(harness.frames.isRunning)
    }

    func testInterruptResolvesCurrentValueAndVelocityWithoutWaitingForAFrame() throws {
        let harness = Harness()
        let curve = GaryxMotionPhysics.SpringCurve(response: 0.34, dampingRatio: 0.82)
        harness.driver.settle(
            from: 120,
            to: 0,
            initialVelocity: 500,
            curve: curve,
            onUpdate: { _ in }
        )

        harness.clock.now = 10.173
        let interrupted = try XCTUnwrap(harness.driver.interrupt())
        let expected = GaryxMotionPhysics.SettleTrajectory(
            initialValue: 120,
            targetValue: 0,
            initialVelocity: 500,
            curve: curve
        ).sample(elapsedTime: 0.173)
        XCTAssertEqual(interrupted.value, expected.value, accuracy: 1e-9)
        XCTAssertEqual(interrupted.velocity, expected.velocity, accuracy: 1e-9)
        XCTAssertFalse(harness.driver.isSettling)
        XCTAssertFalse(harness.frames.isRunning)
    }

    func testFramesUseAbsoluteClockAndCompleteAtExactTarget() throws {
        let harness = Harness()
        let curve = GaryxMotionPhysics.SpringCurve(response: 0.22, dampingRatio: 0.88)
        var samples: [GaryxMotionPhysics.MotionSample] = []
        var completions = 0
        harness.driver.settle(
            from: 40,
            to: 0,
            initialVelocity: 300,
            curve: curve,
            onUpdate: { samples.append($0) },
            onCompletion: { completions += 1 }
        )

        harness.advance(to: 10.1)
        let sampleAtPointOne = try XCTUnwrap(samples.last)
        let expected = GaryxMotionPhysics.SettleTrajectory(
            initialValue: 40,
            targetValue: 0,
            initialVelocity: 300,
            curve: curve
        ).sample(elapsedTime: 0.1)
        XCTAssertEqual(sampleAtPointOne.value, expected.value, accuracy: 1e-9)
        XCTAssertEqual(sampleAtPointOne.velocity, expected.velocity, accuracy: 1e-9)

        harness.advance(to: 10 + curve.settlingDuration)
        XCTAssertEqual(samples.last, .init(value: 0, velocity: 0))
        XCTAssertEqual(completions, 1)
        XCTAssertFalse(harness.frames.isRunning)
    }

    func testInvalidateAndDeinitAlwaysStopFrameCallbacks() {
        let clock = ManualTimeSource(now: 10)
        let frames = ManualFrameSource()
        var driver: GaryxGestureSettleDriver? = GaryxGestureSettleDriver(
            timeSource: clock,
            frameSource: frames
        )
        driver?.settle(
            from: 30,
            to: 0,
            initialVelocity: 100,
            curve: .init(response: 0.22, dampingRatio: 0.88),
            onUpdate: { _ in }
        )
        driver?.invalidate()
        XCTAssertFalse(frames.isRunning)
        XCTAssertNil(frames.onFrame)

        driver?.settle(
            from: 30,
            to: 0,
            initialVelocity: 100,
            curve: .init(response: 0.22, dampingRatio: 0.88),
            onUpdate: { _ in }
        )
        let invalidationsBeforeDeinit = frames.invalidationCount
        driver = nil
        XCTAssertGreaterThan(frames.invalidationCount, invalidationsBeforeDeinit)
        XCTAssertFalse(frames.isRunning)
        XCTAssertNil(frames.onFrame)
    }

    @MainActor
    private final class Harness {
        let clock = ManualTimeSource(now: 10)
        let frames = ManualFrameSource()
        lazy var driver = GaryxGestureSettleDriver(timeSource: clock, frameSource: frames)

        func advance(to time: TimeInterval) {
            clock.now = time
            frames.fire()
        }
    }

    @MainActor
    private final class ManualTimeSource: GaryxGestureSettleTimeSource {
        var now: TimeInterval

        init(now: TimeInterval) {
            self.now = now
        }
    }

    private final class ManualFrameSource: GaryxGestureSettleFrameSource {
        var onFrame: (() -> Void)?
        private(set) var isRunning = false
        private(set) var invalidationCount = 0

        func start() {
            isRunning = true
        }

        func invalidate() {
            invalidationCount += 1
            isRunning = false
        }

        func fire() {
            guard isRunning else { return }
            onFrame?()
        }
    }
}
