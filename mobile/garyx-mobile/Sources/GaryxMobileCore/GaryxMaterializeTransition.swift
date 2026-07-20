public struct GaryxMaterializeTransitionVisualState: Equatable, Sendable {
    public let opacity: Double
    public let scale: Double
    public let blurRadius: Double

    public init(opacity: Double, scale: Double, blurRadius: Double) {
        self.opacity = min(1, max(0, opacity))
        self.scale = max(0, scale)
        self.blurRadius = max(0, blurRadius)
    }

    public static let identity = Self(opacity: 1, scale: 1, blurRadius: 0)
}

/// Accessibility policy for transient material surfaces. Standard motion
/// arrives as one blur + scale + opacity materialization. Cross Fade strips
/// every spatial/filter component, Reduce Motion is immediate, and Reduce
/// Transparency keeps the spatial arrival while removing the blur.
public enum GaryxMaterializeTransitionPolicy {
    public static func activeState(
        transitionMode: GaryxAccessibilityTransitionMode,
        reduceTransparency: Bool,
        initialScale: Double = 0.97,
        initialBlurRadius: Double = 10
    ) -> GaryxMaterializeTransitionVisualState {
        switch transitionMode {
        case .spatial:
            return .init(
                opacity: 0,
                scale: initialScale,
                blurRadius: reduceTransparency ? 0 : initialBlurRadius
            )
        case .crossFade:
            return .init(opacity: 0, scale: 1, blurRadius: 0)
        case .immediate:
            return .identity
        }
    }
}
