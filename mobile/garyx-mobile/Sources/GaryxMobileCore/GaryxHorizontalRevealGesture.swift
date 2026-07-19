import CoreGraphics

/// The two stable endpoints shared by drawers, side rails, and row actions.
public enum GaryxHorizontalRevealPosition: String, Equatable, Sendable {
    case closed
    case open

    public func reveal(for extent: CGFloat) -> CGFloat {
        self == .open ? max(0, extent) : 0
    }
}

/// Explicit interaction ownership replaces view-local "can start" gates.
/// A settling surface remains eligible: a new drag interrupts the analytic
/// trajectory, adopts its current value, and returns to `.dragging`.
public enum GaryxHorizontalRevealPhase: Equatable, Sendable {
    case idle
    case dragging
    case settling(GaryxHorizontalRevealPosition)

    /// Reveal transitions must reject descendant taps, but that interaction
    /// freeze is not a disabled control state. SwiftUI consumers apply this as
    /// a hit-testing gate so existing content never adopts disabled styling
    /// while a drag or programmatic settle is in flight.
    public var allowsSurfaceHitTesting: Bool {
        self == .idle
    }
}

public struct GaryxHorizontalRevealSettle: Equatable, Sendable {
    public let target: GaryxHorizontalRevealPosition
    public let initialReveal: CGFloat
    public let initialVelocity: CGFloat

    public init(
        target: GaryxHorizontalRevealPosition,
        initialReveal: CGFloat,
        initialVelocity: CGFloat
    ) {
        self.target = target
        self.initialReveal = initialReveal
        self.initialVelocity = initialVelocity
    }
}

/// Pure state for one finite horizontal reveal track.
///
/// `reveal` and velocity are in points / points-per-second. Positive movement
/// always means "more open"; UIKit/SwiftUI adapters map physical direction to
/// that logical axis before calling the reducer.
public struct GaryxHorizontalRevealState: Equatable, Sendable {
    public private(set) var settledPosition: GaryxHorizontalRevealPosition
    public private(set) var phase: GaryxHorizontalRevealPhase
    public private(set) var reveal: CGFloat

    private var dragOrigin: CGFloat
    private var cancellationTarget: GaryxHorizontalRevealPosition

    public init(
        position: GaryxHorizontalRevealPosition = .closed,
        extent: CGFloat = 0
    ) {
        settledPosition = position
        phase = .idle
        reveal = position.reveal(for: extent)
        dragOrigin = reveal
        cancellationTarget = position
    }

    public var targetPosition: GaryxHorizontalRevealPosition {
        if case .settling(let target) = phase {
            return target
        }
        return settledPosition
    }

    public var isDragging: Bool {
        phase == .dragging
    }

    public var isSettling: Bool {
        if case .settling = phase { return true }
        return false
    }

    /// Re-derives point geometry without changing the interaction phase.
    public mutating func rederiveExtent(from oldExtent: CGFloat, to newExtent: CGFloat) {
        let oldExtent = max(0, oldExtent)
        let newExtent = max(0, newExtent)
        guard oldExtent != newExtent else { return }
        guard oldExtent > 0 else {
            reveal = settledPosition.reveal(for: newExtent)
            dragOrigin = reveal
            return
        }
        let scale = newExtent / oldExtent
        reveal *= scale
        dragOrigin *= scale
    }

    /// Hard synchronization used at first mount, teardown, and immediate
    /// accessibility transitions.
    public mutating func synchronize(
        to position: GaryxHorizontalRevealPosition,
        extent: CGFloat
    ) {
        settledPosition = position
        phase = .idle
        reveal = position.reveal(for: extent)
        dragOrigin = reveal
        cancellationTarget = position
    }

    /// Starts a new drag or takes over a settle at its analytically sampled
    /// presentation value. The prior settle target becomes the cancellation
    /// destination if UIKit later cancels this new touch stream.
    public mutating func beginDrag(interruptedReveal: CGFloat? = nil, extent: CGFloat) {
        if case .settling(let target) = phase {
            cancellationTarget = target
        } else {
            cancellationTarget = settledPosition
        }
        if let interruptedReveal {
            reveal = Self.rubberBandedReveal(interruptedReveal, extent: extent)
        }
        dragOrigin = reveal
        phase = .dragging
    }

    @discardableResult
    public mutating func updateDrag(
        logicalTranslation: CGFloat,
        extent: CGFloat
    ) -> CGFloat {
        guard phase == .dragging else { return reveal }
        reveal = Self.rubberBandedReveal(
            dragOrigin + logicalTranslation,
            extent: extent
        )
        return reveal
    }

    /// Projects the release landing, then hands the exact release velocity to
    /// the settle driver. The halfway rule is shared by all reveal tracks;
    /// their projection horizon is selected by the caller.
    public mutating func release(
        logicalVelocity: CGFloat,
        extent: CGFloat,
        projection: GaryxMotionPhysics.ProjectionPolicy
    ) -> GaryxHorizontalRevealSettle? {
        guard phase == .dragging, extent > 0 else { return nil }
        let projected = projection.projectedValue(
            valuePoints: reveal,
            velocityPointsPerSecond: logicalVelocity
        )
        let target: GaryxHorizontalRevealPosition = projected > extent * 0.5
            ? .open
            : .closed
        phase = .settling(target)
        return GaryxHorizontalRevealSettle(
            target: target,
            initialReveal: reveal,
            initialVelocity: logicalVelocity
        )
    }

    /// UIKit cancellation resumes the endpoint that owned the surface when the
    /// current touch began. This is an explicit reducer event, not liveness
    /// inferred from a SwiftUI `GestureState` reset.
    public mutating func cancelDrag(extent: CGFloat) -> GaryxHorizontalRevealSettle? {
        guard phase == .dragging else { return nil }
        let target = cancellationTarget
        phase = .settling(target)
        return GaryxHorizontalRevealSettle(
            target: target,
            initialReveal: reveal,
            initialVelocity: 0
        )
    }

    public mutating func beginProgrammaticSettle(
        to target: GaryxHorizontalRevealPosition,
        initialVelocity: CGFloat,
        extent: CGFloat
    ) -> GaryxHorizontalRevealSettle? {
        guard extent > 0 else { return nil }
        if phase == .idle, settledPosition == target {
            reveal = target.reveal(for: extent)
            return nil
        }
        phase = .settling(target)
        cancellationTarget = target
        return GaryxHorizontalRevealSettle(
            target: target,
            initialReveal: reveal,
            initialVelocity: initialVelocity
        )
    }

    @discardableResult
    public mutating func updateSettle(sampledReveal: CGFloat, extent: CGFloat) -> CGFloat {
        guard isSettling else { return reveal }
        reveal = Self.rubberBandedReveal(sampledReveal, extent: extent)
        return reveal
    }

    public mutating func finishSettle(extent: CGFloat) -> GaryxHorizontalRevealPosition? {
        guard case .settling(let target) = phase else { return nil }
        synchronize(to: target, extent: extent)
        return target
    }

    public static func rubberBandedReveal(_ rawReveal: CGFloat, extent: CGFloat) -> CGFloat {
        let extent = max(0, extent)
        guard extent > 0 else { return 0 }
        if rawReveal < 0 {
            return GaryxMotionPhysics.rubberband(
                overshoot: rawReveal,
                dimension: extent
            )
        }
        if rawReveal > extent {
            return extent + GaryxMotionPhysics.rubberband(
                overshoot: rawReveal - extent,
                dimension: extent
            )
        }
        return rawReveal
    }
}
