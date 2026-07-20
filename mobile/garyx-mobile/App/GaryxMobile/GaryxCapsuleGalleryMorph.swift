import SwiftUI

struct GaryxCapsuleGalleryThumbnailAnchorKey: PreferenceKey {
    static var defaultValue: [String: Anchor<CGRect>] = [:]

    static func reduce(
        value: inout [String: Anchor<CGRect>],
        nextValue: () -> [String: Anchor<CGRect>]
    ) {
        value.merge(nextValue(), uniquingKeysWith: { _, next in next })
    }
}

/// One clipped full-canvas surface whose bounds grow from the tapped gallery
/// thumbnail. `progress` is animatable data, so both directions use the exact
/// same Core geometry at every sampled frame instead of handing off between
/// separately mounted source and destination views.
struct GaryxCapsuleGalleryMorphSurface: View, Animatable {
    let selection: GaryxCapsulePreviewSelection
    let sourceRect: CGRect
    let containerSize: CGSize
    let onDismiss: () -> Void
    var progress: CGFloat

    var animatableData: CGFloat {
        get { progress }
        set { progress = newValue }
    }

    var body: some View {
        let layout = GaryxAnchoredFullscreenMorphGeometry.layout(
            progress: progress,
            sourceRect: sourceRect,
            containerSize: containerSize
        )
        let shape = RoundedRectangle(cornerRadius: layout.cornerRadius, style: .continuous)

        GaryxCapsuleFocusedPreviewView(
            selection: selection,
            onRequestDismiss: onDismiss
        )
        // Keep the destination at its final canvas size while the outer window
        // grows. That prevents the web preview and top chrome from relaying out
        // on every animation frame.
        .frame(width: containerSize.width, height: containerSize.height)
        .opacity(layout.contentOpacity)
        .frame(
            width: layout.frame.width,
            height: layout.frame.height,
            alignment: .topLeading
        )
        .clipShape(shape)
        .contentShape(shape)
        .shadow(
            color: Color.black.opacity(0.16 * Double(progress)),
            radius: 28 * progress,
            x: 0,
            y: 12 * progress
        )
        .offset(x: layout.frame.minX, y: layout.frame.minY)
        .accessibilityAddTraits(.isModal)
        .accessibilityAction(.escape, onDismiss)
    }
}
