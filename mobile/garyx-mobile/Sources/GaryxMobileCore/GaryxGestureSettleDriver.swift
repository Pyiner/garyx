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

    private struct SupplementalFrameObserver {
        let isActive: () -> Bool
        let onFrame: () -> Void
    }

    private struct ActiveSettle {
        let trajectory: GaryxMotionPhysics.SettleTrajectory
        let startTime: TimeInterval
        let onUpdate: (MotionSample) -> Void
        let onCompletion: () -> Void
    }

    private let timeSource: any GaryxGestureSettleTimeSource
    private let frameSource: any GaryxGestureSettleFrameSource
    private var activeSettle: ActiveSettle?
    private var supplementalFrameObserver: SupplementalFrameObserver?
    private var isFrameSourceRunning = false

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

    /// Shares the driver's existing presented-frame source with lightweight
    /// work that must begin immediately after a settle reaches terminal.
    ///
    /// The observer is deliberately demand-driven. If `isActive` becomes true
    /// from the settle completion callback, the current frame source remains
    /// installed instead of invalidating and recreating a CADisplayLink at the
    /// navigation boundary.
    public func setSupplementalFrameObserver(
        isActive: @escaping () -> Bool,
        onFrame: @escaping () -> Void
    ) {
        supplementalFrameObserver = SupplementalFrameObserver(
            isActive: isActive,
            onFrame: onFrame
        )
    }

    /// Starts the shared frame source when supplemental work becomes active
    /// without an in-flight settle, such as an immediate or initially mounted
    /// route.
    public func ensureSupplementalFrames() {
        guard supplementalFrameObserver?.isActive() == true else { return }
        startFrameSourceIfNeeded()
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
        startFrameSourceIfNeeded()
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
        if supplementalFrameObserver?.isActive() == true {
            supplementalFrameObserver?.onFrame()
        }

        guard let activeSettle else {
            stopFrameSourceIfIdle()
            return
        }
        let elapsedTime = max(0, timeSource.now - activeSettle.startTime)
        if elapsedTime >= activeSettle.trajectory.settlingDuration {
            let finalSample = MotionSample(
                value: activeSettle.trajectory.targetValue,
                velocity: 0
            )
            self.activeSettle = nil
            activeSettle.onUpdate(finalSample)
            activeSettle.onCompletion()
            stopFrameSourceIfIdle()
            return
        }
        activeSettle.onUpdate(activeSettle.trajectory.sample(elapsedTime: elapsedTime))
    }

    private func sample(_ settle: ActiveSettle, now: TimeInterval) -> MotionSample {
        settle.trajectory.sample(elapsedTime: max(0, now - settle.startTime))
    }

    private func clearActiveSettle() {
        activeSettle = nil
        stopFrameSource()
    }

    private func startFrameSourceIfNeeded() {
        guard !isFrameSourceRunning else { return }
        frameSource.onFrame = { [weak self] in
            self?.handleFrame()
        }
        frameSource.start()
        isFrameSourceRunning = true
    }

    private func stopFrameSourceIfIdle() {
        guard activeSettle == nil,
              supplementalFrameObserver?.isActive() != true
        else { return }
        stopFrameSource()
    }

    private func stopFrameSource() {
        guard isFrameSourceRunning || frameSource.onFrame != nil else { return }
        frameSource.onFrame = nil
        frameSource.invalidate()
        isFrameSourceRunning = false
    }
}
