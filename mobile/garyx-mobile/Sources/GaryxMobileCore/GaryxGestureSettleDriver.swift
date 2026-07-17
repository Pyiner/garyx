import CoreGraphics
import Foundation

/// Monotonic time in seconds, injected so settle behavior is deterministic in
/// Core tests.
@MainActor
public protocol GaryxGestureSettleTimeSource: AnyObject {
    var now: TimeInterval { get }
}

/// A reusable frame callback source. The iOS app supplies a CADisplayLink
/// implementation; Core tests supply a manually advanced source.
public protocol GaryxGestureSettleFrameSource: AnyObject {
    var onFrame: (() -> Void)? { get set }
    func start()
    func invalidate()
}

/// Refresh-rate-independent driver for one scalar spring settle.
///
/// The driver owns lifecycle and callback orchestration only. Every frame and
/// interruption resolves the Core analytic trajectory at absolute elapsed
/// time, preserving velocity without a frame-by-frame integrator.
@MainActor
public final class GaryxGestureSettleDriver {
    public typealias MotionSample = GaryxMotionPhysics.MotionSample

    private struct ActiveSettle {
        let trajectory: GaryxMotionPhysics.SettleTrajectory
        let startTime: TimeInterval
        let onUpdate: (MotionSample) -> Void
        let onCompletion: () -> Void
    }

    private let timeSource: any GaryxGestureSettleTimeSource
    private let frameSource: any GaryxGestureSettleFrameSource
    private var activeSettle: ActiveSettle?

    public init(
        timeSource: any GaryxGestureSettleTimeSource,
        frameSource: any GaryxGestureSettleFrameSource
    ) {
        self.timeSource = timeSource
        self.frameSource = frameSource
    }

    public var isSettling: Bool {
        activeSettle != nil
    }

    public func settle(
        from initialValue: CGFloat,
        to targetValue: CGFloat,
        initialVelocity: CGFloat,
        curve: GaryxMotionPhysics.SpringCurve,
        onUpdate: @escaping (MotionSample) -> Void,
        onCompletion: @escaping () -> Void = {}
    ) {
        invalidate()
        let trajectory = GaryxMotionPhysics.SettleTrajectory(
            initialValue: initialValue,
            targetValue: targetValue,
            initialVelocity: initialVelocity,
            curve: curve
        )
        activeSettle = ActiveSettle(
            trajectory: trajectory,
            startTime: timeSource.now,
            onUpdate: onUpdate,
            onCompletion: onCompletion
        )
        onUpdate(trajectory.sample(elapsedTime: 0))
        frameSource.onFrame = { [weak self] in
            self?.handleFrame()
        }
        frameSource.start()
    }

    /// Stops the active settle and analytically resolves its current physical
    /// state, including velocity at the interruption instant.
    @discardableResult
    public func interrupt() -> (value: CGFloat, velocity: CGFloat)? {
        guard let activeSettle else { return nil }
        let sample = sample(activeSettle, now: timeSource.now)
        clearActiveSettle()
        return (sample.value, sample.velocity)
    }

    /// Cancels callbacks and releases the frame source without completing the
    /// trajectory. Safe to call for scene deactivation and view teardown.
    public func invalidate() {
        clearActiveSettle()
    }

    deinit {
        frameSource.onFrame = nil
        frameSource.invalidate()
    }

    private func handleFrame() {
        guard let activeSettle else { return }
        let elapsedTime = max(0, timeSource.now - activeSettle.startTime)
        if elapsedTime >= activeSettle.trajectory.settlingDuration {
            let finalSample = MotionSample(
                value: activeSettle.trajectory.targetValue,
                velocity: 0
            )
            clearActiveSettle()
            activeSettle.onUpdate(finalSample)
            activeSettle.onCompletion()
            return
        }
        activeSettle.onUpdate(activeSettle.trajectory.sample(elapsedTime: elapsedTime))
    }

    private func sample(_ settle: ActiveSettle, now: TimeInterval) -> MotionSample {
        settle.trajectory.sample(elapsedTime: max(0, now - settle.startTime))
    }

    private func clearActiveSettle() {
        activeSettle = nil
        frameSource.onFrame = nil
        frameSource.invalidate()
    }
}
