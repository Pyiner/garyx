import SwiftUI

/// Shared visual primitive for an in-place chrome morph. The caller owns all
/// presentation wiring and timing; this view owns only the two pinned frames and
/// the single glass surface recipe.
struct GaryxChromeMorphSurface<Content: View>: View {
    let isExpanded: Bool
    let anchorRect: CGRect
    let containerSize: CGSize
    let metrics: GaryxChromeMorphSurfaceMetrics
    let onClose: () -> Void
    private let content: Content

    init(
        isExpanded: Bool,
        anchorRect: CGRect,
        containerSize: CGSize,
        metrics: GaryxChromeMorphSurfaceMetrics,
        onClose: @escaping () -> Void,
        @ViewBuilder content: () -> Content
    ) {
        self.isExpanded = isExpanded
        self.anchorRect = anchorRect
        self.containerSize = containerSize
        self.metrics = metrics
        self.onClose = onClose
        self.content = content()
    }

    var body: some View {
        let layout = GaryxChromeMorphSurfaceGeometry.layout(
            isExpanded: isExpanded,
            anchorRect: anchorRect,
            containerSize: containerSize,
            metrics: metrics
        )
        let shape = RoundedRectangle(cornerRadius: layout.cornerRadius, style: .continuous)

        content
            // Keep content at its final width while only the outer clipping
            // window morphs, preventing title/body reflow.
            .frame(width: layout.expandedWidth, alignment: .topLeading)
            .frame(
                width: layout.outerWidth,
                height: layout.outerHeight,
                alignment: .topLeading
            )
            .background {
                shape.fill(Color(.systemBackground).opacity(isExpanded ? 0.72 : 0))
            }
            // Liquid Glass hard contract: glass is applied directly to the
            // content view. Never move it into a GlassEffectContainer
            // `.background`, where iOS can hoist it above its own content.
            .garyxAdaptiveGlass(
                .regular,
                isInteractive: false,
                fallbackMaterial: .ultraThinMaterial,
                in: shape
            )
            .clipShape(shape)
            .contentShape(shape)
            .overlay {
                shape
                    .stroke(Color.white.opacity(0.30), lineWidth: 0.7)
                    .opacity(isExpanded ? 1 : 0)
            }
            .overlay {
                shape
                    .stroke(Color.primary.opacity(0.06), lineWidth: 1)
                    .opacity(isExpanded ? 1 : 0)
            }
            .shadow(
                color: Color.black.opacity(isExpanded ? 0.10 : 0),
                radius: 24,
                x: 0,
                y: 10
            )
            .offset(x: layout.outerX, y: layout.outerY)
            .accessibilityAddTraits(.isModal)
            .accessibilityAction(.escape, onClose)
    }
}
