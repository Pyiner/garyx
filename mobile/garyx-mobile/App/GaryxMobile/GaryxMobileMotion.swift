import SwiftUI

/// SwiftUI adapter for the pure Core motion catalog. Views consume this
/// environment value instead of branching on Reduce Motion or cross-fade
/// preferences themselves.
struct GaryxMotionContext: Equatable {
    let preferences: GaryxMotion.Preferences

    static let standard = Self(preferences: .standard)

    func resolution(_ token: GaryxMotion.Token) -> GaryxMotion.Resolution {
        GaryxMotion.resolve(token, preferences: preferences)
    }

    /// Animation for opacity-safe state changes and token-owned transitions.
    func animation(_ token: GaryxMotion.Token) -> Animation? {
        resolution(token).curve?.swiftUIAnimation
    }

    /// Animation for geometry, scrolling, scale, and other spatial movement.
    func spatialAnimation(_ token: GaryxMotion.Token) -> Animation? {
        let resolution = resolution(token)
        guard resolution.allowsSpatialMotion else { return nil }
        return resolution.curve?.swiftUIAnimation
    }

    func animates(_ token: GaryxMotion.Token) -> Bool {
        resolution(token).animates
    }

    func animatesSpatially(_ token: GaryxMotion.Token) -> Bool {
        spatialAnimation(token) != nil
    }

    func allowsSpatialMotion(_ token: GaryxMotion.Token) -> Bool {
        resolution(token).allowsSpatialMotion
    }

    func scale(_ token: GaryxMotion.Token, active: Bool) -> CGFloat {
        guard active else { return 1 }
        return CGFloat(resolution(token).effect.scale)
    }

    func opacity(_ token: GaryxMotion.Token, active: Bool) -> Double {
        guard active else { return 1 }
        return resolution(token).effect.opacity
    }

    func offset(_ token: GaryxMotion.Token, active: Bool) -> CGSize {
        guard active else { return .zero }
        let effect = resolution(token).effect
        return CGSize(width: CGFloat(effect.offsetX), height: CGFloat(effect.offsetY))
    }

    func cycleDuration(_ token: GaryxMotion.Token) -> TimeInterval {
        GaryxMotion.specification(for: token).curve.duration
    }

    func continuousAnimation(_ token: GaryxMotion.Token) -> Animation {
        let specification = GaryxMotion.specification(for: token)
        precondition(specification.kinetics == .continuous)
        return specification.curve.swiftUIAnimation
    }

    func pausesContinuousMotion(_ token: GaryxMotion.Token) -> Bool {
        let specification = GaryxMotion.specification(for: token)
        guard specification.kinetics == .continuous else { return false }
        return !resolution(token).allowsSpatialMotion
    }

    /// Creates a token-owned opacity transition, optionally combined with a
    /// directional move. Cross-fade and immediate policies strip all spatial
    /// components centrally.
    func transition(
        _ token: GaryxMotion.Token,
        moveFrom edge: Edge? = nil,
        anchor: UnitPoint = .center
    ) -> AnyTransition {
        let resolution = resolution(token)
        switch resolution.mode {
        case .immediate:
            return .identity
        case .crossFade:
            return .opacity
        case .spatial:
            break
        }

        var transition = AnyTransition.opacity
        if let edge {
            transition = transition.combined(with: .move(edge: edge))
        }
        if resolution.effect.scale != 1 {
            transition = transition.combined(
                with: .scale(scale: resolution.effect.scale, anchor: anchor)
            )
        }
        if resolution.effect.offsetX != 0 || resolution.effect.offsetY != 0 {
            transition = transition.combined(
                with: .offset(
                    x: CGFloat(resolution.effect.offsetX),
                    y: CGFloat(resolution.effect.offsetY)
                )
            )
        }
        return transition
    }
}

private struct GaryxMotionContextKey: EnvironmentKey {
    static let defaultValue = GaryxMotionContext.standard
}

extension EnvironmentValues {
    var garyxMotion: GaryxMotionContext {
        get { self[GaryxMotionContextKey.self] }
        set { self[GaryxMotionContextKey.self] = newValue }
    }
}

private extension GaryxMotion.Curve {
    var swiftUIAnimation: Animation {
        switch self {
        case let .spring(response, dampingRatio):
            .spring(response: response, dampingFraction: dampingRatio)
        case let .easeIn(duration):
            .easeIn(duration: duration)
        case let .easeOut(duration):
            .easeOut(duration: duration)
        case let .easeInOut(duration):
            .easeInOut(duration: duration)
        case let .timingCurve(x1, y1, x2, y2, duration):
            .timingCurve(x1, y1, x2, y2, duration: duration)
        case let .linear(duration):
            .linear(duration: duration)
        }
    }
}
