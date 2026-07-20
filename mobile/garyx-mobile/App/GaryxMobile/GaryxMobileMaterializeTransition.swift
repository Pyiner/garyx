import SwiftUI

private struct GaryxMaterializeVisualModifier: ViewModifier {
    let state: GaryxMaterializeTransitionVisualState
    let anchor: UnitPoint

    func body(content: Content) -> some View {
        content
            .scaleEffect(CGFloat(state.scale), anchor: anchor)
            .blur(radius: CGFloat(state.blurRadius))
            .opacity(state.opacity)
    }
}

private struct GaryxMaterializeTransitionModifier: ViewModifier {
    @Environment(\.garyxMotion) private var motion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    let token: GaryxMotion.Token
    let anchor: UnitPoint
    let initialScale: CGFloat?
    let initialBlurRadius: CGFloat

    func body(content: Content) -> some View {
        let resolution = motion.resolution(token)
        let tokenScale = CGFloat(resolution.effect.scale)
        let resolvedScale = initialScale ?? (tokenScale == 1 ? 0.97 : tokenScale)
        let activeState = GaryxMaterializeTransitionPolicy.activeState(
            transitionMode: resolution.mode,
            reduceTransparency: reduceTransparency,
            initialScale: Double(resolvedScale),
            initialBlurRadius: Double(initialBlurRadius)
        )

        content.transition(
            .modifier(
                active: GaryxMaterializeVisualModifier(
                    state: activeState,
                    anchor: anchor
                ),
                identity: GaryxMaterializeVisualModifier(
                    state: .identity,
                    anchor: anchor
                )
            )
        )
    }
}

extension View {
    /// Gives a transient material surface a symmetric arrival/departure path.
    /// Accessibility fallbacks are selected by the shared Core policy.
    func garyxMaterializeTransition(
        _ token: GaryxMotion.Token,
        anchor: UnitPoint = .center,
        initialScale: CGFloat? = nil,
        initialBlurRadius: CGFloat = 10
    ) -> some View {
        modifier(
            GaryxMaterializeTransitionModifier(
                token: token,
                anchor: anchor,
                initialScale: initialScale,
                initialBlurRadius: initialBlurRadius
            )
        )
    }
}
