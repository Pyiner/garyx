import SwiftUI
import UIKit

// MARK: - In-place message action menu
//
// The system `.contextMenu` lifts a snapshot of the pressed row: the message
// visibly moves, scales, and gains a preview background while the menu is up.
// Transcript content must do the opposite — a long-pressed message stays
// exactly where it is, in its original size and style, and only a floating
// action panel appears next to it.
//
// Rows publish their bounds plus menu items through an anchor preference when
// long-pressed; a host overlay attached at the surface root (outside the
// scroll content, so the panel is never clipped) resolves the anchor and
// renders the panel. Nothing about the pressed row itself changes.

struct GaryxMessageMenuItem: Identifiable {
    let id = UUID()
    let title: String
    let systemImage: String
    let handler: () -> Void
}

enum GaryxMessageMenuEdge {
    case leading
    case trailing
}

struct GaryxMessageMenuRequest: Equatable {
    let token: UUID
    let anchor: Anchor<CGRect>
    let edge: GaryxMessageMenuEdge
    let items: [GaryxMessageMenuItem]
    let dismiss: () -> Void

    static func == (lhs: GaryxMessageMenuRequest, rhs: GaryxMessageMenuRequest) -> Bool {
        lhs.token == rhs.token
    }
}

struct GaryxMessageMenuPreferenceKey: PreferenceKey {
    static let defaultValue: GaryxMessageMenuRequest? = nil

    static func reduce(
        value: inout GaryxMessageMenuRequest?,
        nextValue: () -> GaryxMessageMenuRequest?
    ) {
        value = value ?? nextValue()
    }
}

extension View {
    /// Long-press action menu that keeps the pressed content completely
    /// static. `items` is evaluated when the press fires; returning an empty
    /// array suppresses the menu.
    func garyxInPlaceMessageMenu(
        edge: GaryxMessageMenuEdge = .leading,
        items: @escaping () -> [GaryxMessageMenuItem]
    ) -> some View {
        modifier(GaryxInPlaceMessageMenuModifier(edge: edge, itemsProvider: items))
    }

    /// Hosts the floating panels published by `garyxInPlaceMessageMenu` rows
    /// below this view. Attach once at the scrollable surface root.
    /// `bottomInset` keeps panels above floating bottom chrome.
    func garyxMessageMenuHost(bottomInset: CGFloat = 0, dismissToken: String = "") -> some View {
        modifier(GaryxMessageMenuHostModifier(bottomInset: bottomInset, dismissToken: dismissToken))
    }
}

private struct GaryxInPlaceMessageMenuModifier: ViewModifier {
    let edge: GaryxMessageMenuEdge
    let itemsProvider: () -> [GaryxMessageMenuItem]

    @State private var presented: PresentedMenu?

    private struct PresentedMenu {
        let token: UUID
        let items: [GaryxMessageMenuItem]
    }

    func body(content: Content) -> some View {
        content
            // High priority so a recognized long-press cancels taps on inner
            // buttons/links; quick taps fail the long-press and pass through.
            .highPriorityGesture(
                LongPressGesture(minimumDuration: 0.35)
                    .onEnded { _ in
                        let items = itemsProvider()
                        guard !items.isEmpty else { return }
                        GaryxMobileHaptics.shared.prepare(.clipboardCopySucceeded)
                        GaryxMobileHaptics.shared.play(.messageActionMenuPresented)
                        presented = PresentedMenu(token: UUID(), items: items)
                    }
            )
            .anchorPreference(key: GaryxMessageMenuPreferenceKey.self, value: .bounds) { anchor in
                guard let presented else { return nil }
                return GaryxMessageMenuRequest(
                    token: presented.token,
                    anchor: anchor,
                    edge: edge,
                    items: presented.items,
                    dismiss: { self.presented = nil }
                )
            }
    }
}

private struct GaryxMessageMenuHostModifier: ViewModifier {
    var bottomInset: CGFloat = 0
    var dismissToken = ""
    @Environment(\.garyxMotion) private var motion
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @ScaledMetric(relativeTo: .subheadline) private var rowHeight: CGFloat = 46

    private static let margin: CGFloat = 12

    func body(content: Content) -> some View {
        content.overlayPreferenceValue(GaryxMessageMenuPreferenceKey.self) { request in
            GeometryReader { proxy in
                ZStack(alignment: .topLeading) {
                    if let request {
                        Color.clear
                            .contentShape(Rectangle())
                            .onTapGesture { request.dismiss() }

                        GaryxMessageMenuPanel(request: request)
                            .frame(width: menuWidth(in: proxy.size.width))
                            .offset(panelOffset(for: request, in: proxy))
                            .garyxMaterializeTransition(
                                .messageMenu,
                                anchor: materializeAnchor(for: request)
                            )
                    }
                }
                .animation(motion.animation(.messageMenu), value: request?.token)
                .onChange(of: dismissToken) { _, _ in
                    request?.dismiss()
                }
            }
        }
    }

    private func panelOffset(for request: GaryxMessageMenuRequest, in proxy: GeometryProxy) -> CGSize {
        let rect = proxy[request.anchor]
        let size = proxy.size
        let resolvedMenuWidth = menuWidth(in: size.width)
        let menuHeight = CGFloat(request.items.count) * rowHeight + 12

        var x: CGFloat
        switch request.edge {
        case .leading: x = rect.minX
        case .trailing: x = rect.maxX - resolvedMenuWidth
        }
        x = min(max(x, Self.margin), max(Self.margin, size.width - resolvedMenuWidth - Self.margin))

        let bottomLimit = size.height - bottomInset - Self.margin
        var y = rect.maxY + 8
        if y + menuHeight > bottomLimit {
            y = rect.minY - menuHeight - 8
        }
        y = min(max(y, Self.margin), max(Self.margin, bottomLimit - menuHeight))
        return CGSize(width: x, height: y)
    }

    private func menuWidth(in availableWidth: CGFloat) -> CGFloat {
        let preferredWidth: CGFloat = dynamicTypeSize.garyxUsesExpandedReadingLayout ? 340 : 236
        return min(preferredWidth, availableWidth - Self.margin * 2)
    }

    private func materializeAnchor(for request: GaryxMessageMenuRequest) -> UnitPoint {
        switch request.edge {
        case .leading: .topLeading
        case .trailing: .topTrailing
        }
    }
}

private struct GaryxMessageMenuPanel: View {
    @ScaledMetric(relativeTo: .subheadline) private var rowHeight: CGFloat = 46
    @ScaledMetric(relativeTo: .subheadline) private var verticalPadding: CGFloat = 8
    let request: GaryxMessageMenuRequest

    var body: some View {
        GaryxGlassPanel(cornerRadius: 18, shadowOpacity: 0.16) {
            VStack(spacing: 0) {
                ForEach(Array(request.items.enumerated()), id: \.element.id) { index, item in
                    if index > 0 {
                        Divider()
                            .opacity(0.6)
                            .padding(.leading, 14)
                    }
                    Button {
                        request.dismiss()
                        item.handler()
                    } label: {
                        HStack {
                            Text(item.title)
                                .font(GaryxFont.subheadline(weight: .medium))
                                .foregroundStyle(.primary)
                            Spacer(minLength: 12)
                            Image(systemName: item.systemImage)
                                .font(GaryxFont.fixedSystem(size: 15, weight: .regular))
                                .foregroundStyle(.secondary)
                        }
                        .padding(.horizontal, 14)
                        .padding(.vertical, verticalPadding)
                        .frame(minHeight: rowHeight)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(GaryxPressableRowStyle())
                }
            }
            .padding(.vertical, 6)
        }
    }
}

// MARK: - In-place thread action menu

/// Compact Home-row menu matched to the in-list long-press treatment: the
/// source row stays in place and receives a focused card treatment while the
/// rest of the surface recedes behind a light scrim.
enum GaryxThreadActionMenuRole {
    case standard
    case destructive
}

struct GaryxThreadActionMenuItem: Identifiable {
    let id: String
    let title: String
    let systemImage: String
    let role: GaryxThreadActionMenuRole
    let isEnabled: Bool
    let handler: () -> Void

    init(
        title: String,
        systemImage: String,
        role: GaryxThreadActionMenuRole = .standard,
        isEnabled: Bool = true,
        handler: @escaping () -> Void
    ) {
        self.id = title
        self.title = title
        self.systemImage = systemImage
        self.role = role
        self.isEnabled = isEnabled
        self.handler = handler
    }
}

private enum GaryxThreadActionMenuMetrics {
    static let menuWidthFraction: CGFloat = 0.565
    static let minimumMenuWidth: CGFloat = 212
    static let maximumMenuWidth: CGFloat = 244
    static let rowHeight: CGFloat = 44
    static let verticalPadding: CGFloat = 6
    static let cornerRadius: CGFloat = 22
    static let surfaceMargin: CGFloat = 18
    static let sourceHorizontalInset: CGFloat = 18
    static let sourceVerticalInset: CGFloat = 2
    static let sourceCornerRadius: CGFloat = 14
    static let panelGap: CGFloat = 16
}

private struct GaryxThreadActionMenuRequest: Equatable {
    let token: UUID
    let anchor: Anchor<CGRect>
    let items: [GaryxThreadActionMenuItem]
    let dismiss: () -> Void

    static func == (lhs: GaryxThreadActionMenuRequest, rhs: GaryxThreadActionMenuRequest) -> Bool {
        lhs.token == rhs.token
    }
}

private struct GaryxThreadActionMenuPreferenceKey: PreferenceKey {
    static let defaultValue: GaryxThreadActionMenuRequest? = nil

    static func reduce(
        value: inout GaryxThreadActionMenuRequest?,
        nextValue: () -> GaryxThreadActionMenuRequest?
    ) {
        value = value ?? nextValue()
    }
}

extension View {
    func garyxThreadActionMenu(
        dismissToken: Int = 0,
        movementSuppressesMenu: Bool = false,
        preparePrimaryAction: @escaping () -> Void = {},
        primaryAction: @escaping () -> Void,
        items: @escaping () -> [GaryxThreadActionMenuItem]
    ) -> some View {
        modifier(
            GaryxThreadActionMenuModifier(
                dismissToken: dismissToken,
                movementSuppressesMenu: movementSuppressesMenu,
                preparePrimaryAction: preparePrimaryAction,
                primaryAction: primaryAction,
                itemsProvider: items
            )
        )
    }

    func garyxThreadActionMenuHost(bottomInset: CGFloat = 0) -> some View {
        modifier(GaryxThreadActionMenuHostModifier(bottomInset: bottomInset))
    }
}

private struct GaryxThreadActionMenuModifier: ViewModifier {
    let dismissToken: Int
    let movementSuppressesMenu: Bool
    let preparePrimaryAction: () -> Void
    let primaryAction: () -> Void
    let itemsProvider: () -> [GaryxThreadActionMenuItem]

    @Environment(\.garyxMotion) private var motion
    @Environment(\.isEnabled) private var isEnabled
    @State private var presented: PresentedMenu?

    private struct PresentedMenu {
        let token: UUID
        let items: [GaryxThreadActionMenuItem]
    }

    func body(content: Content) -> some View {
        interactionSurface(content)
            .background {
                if presented != nil {
                    focusedSourceBackground
                        .transition(motion.transition(.threadMenuFocus))
                }
            }
            .animation(motion.animation(.threadMenuFocus), value: presented?.token)
            .onChange(of: dismissToken) { _, _ in
                presented = nil
            }
            .anchorPreference(key: GaryxThreadActionMenuPreferenceKey.self, value: .bounds) { anchor in
                guard let presented else { return nil }
                return GaryxThreadActionMenuRequest(
                    token: presented.token,
                    anchor: anchor,
                    items: presented.items,
                    dismiss: { self.presented = nil }
                )
            }
            .accessibilityActions {
                ForEach(itemsProvider().filter(\.isEnabled)) { item in
                    Button(item.title, action: item.handler)
                }
            }
    }

    @ViewBuilder
    private func interactionSurface(_ content: Content) -> some View {
        if movementSuppressesMenu {
            // iOS 26's native List reorder recognizer can win an exclusive
            // long-press before the row gesture fires. Arm a stationary-only
            // recognizer alongside it instead: crossing the movement allowance
            // fails this menu gesture and leaves the native lift in control.
            content
                .gesture(
                    GaryxStationaryThreadMenuGesture(onRecognized: presentMenu)
                )
                .simultaneousGesture(TapGesture().onEnded(primaryAction))
                .gesture(primaryPreparationGesture)
        } else {
            // Keep this gesture simultaneous with the List's pan recognizer so
            // scrolling still wins after movement. Inside the row, however,
            // long press and tap are exclusive: once the menu gesture succeeds,
            // releasing the same touch can never also open the thread.
            content
                .simultaneousGesture(primaryInteractionGesture)
                .gesture(primaryPreparationGesture)
        }
    }

    private func presentMenu() {
        guard isEnabled else { return }
        let items = itemsProvider()
        guard items.contains(where: \.isEnabled) else { return }
        GaryxMobileHaptics.shared.prepare(.threadPinChanged)
        GaryxMobileHaptics.shared.play(.messageActionMenuPresented)
        presented = PresentedMenu(token: UUID(), items: items)
    }

    private var primaryInteractionGesture: some Gesture {
        LongPressGesture(minimumDuration: 0.36, maximumDistance: 16)
            .exclusively(before: TapGesture())
            .onEnded { result in
                guard isEnabled else { return }
                switch result {
                case .first:
                    presentMenu()
                case .second:
                    primaryAction()
                }
            }
    }

    /// Observe a short stationary hold before a normal tap releases. UIKit's
    /// simultaneous recognizer is intentional here: SwiftUI's LongPressGesture
    /// can make the List pan wait for this 45 ms preparation gesture to fail.
    /// The action only prepares an inactive route host; navigation remains
    /// owned by the existing tap/long-press arbitration above.
    private var primaryPreparationGesture: GaryxStationaryThreadPreparationGesture {
        GaryxStationaryThreadPreparationGesture {
            guard isEnabled else { return }
            preparePrimaryAction()
        }
    }

    private var focusedSourceBackground: some View {
        // Keep the selected row flat: brightness and a hairline are enough to
        // express focus here; a card shadow makes the in-list row look detached.
        RoundedRectangle(
            cornerRadius: GaryxThreadActionMenuMetrics.sourceCornerRadius,
            style: .continuous
        )
        .fill(Color(.systemBackground))
        .overlay {
            RoundedRectangle(
                cornerRadius: GaryxThreadActionMenuMetrics.sourceCornerRadius,
                style: .continuous
            )
            .stroke(Color.primary.opacity(0.055), lineWidth: 0.7)
        }
        .padding(.horizontal, GaryxThreadActionMenuMetrics.sourceHorizontalInset)
        .padding(.vertical, GaryxThreadActionMenuMetrics.sourceVerticalInset)
    }

}

private struct GaryxStationaryThreadMenuGesture: UIGestureRecognizerRepresentable {
    var onRecognized: () -> Void

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var onRecognized: () -> Void

        init(onRecognized: @escaping () -> Void) {
            self.onRecognized = onRecognized
        }

        func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
        ) -> Bool {
            true
        }
    }

    func makeCoordinator(converter _: CoordinateSpaceConverter) -> Coordinator {
        Coordinator(onRecognized: onRecognized)
    }

    func makeUIGestureRecognizer(context: Context) -> UILongPressGestureRecognizer {
        let recognizer = UILongPressGestureRecognizer()
        recognizer.minimumPressDuration = 0.36
        recognizer.allowableMovement = 16
        recognizer.cancelsTouchesInView = false
        recognizer.delegate = context.coordinator
        return recognizer
    }

    func updateUIGestureRecognizer(
        _ recognizer: UILongPressGestureRecognizer,
        context: Context
    ) {
        context.coordinator.onRecognized = onRecognized
    }

    func handleUIGestureRecognizerAction(
        _ recognizer: UILongPressGestureRecognizer,
        context: Context
    ) {
        guard recognizer.state == .began else { return }
        context.coordinator.onRecognized()
    }
}

/// A passive, simultaneous touch observer for route preparation. In
/// particular, `cancelsTouchesInView = false` and the delegate's simultaneous
/// answer keep a recognized hold from stealing the native List pan when a
/// finger begins moving after the preparation threshold.
private struct GaryxStationaryThreadPreparationGesture: UIGestureRecognizerRepresentable {
    var onRecognized: () -> Void

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var onRecognized: () -> Void

        init(onRecognized: @escaping () -> Void) {
            self.onRecognized = onRecognized
        }

        func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
        ) -> Bool {
            true
        }
    }

    func makeCoordinator(converter _: CoordinateSpaceConverter) -> Coordinator {
        Coordinator(onRecognized: onRecognized)
    }

    func makeUIGestureRecognizer(context: Context) -> UILongPressGestureRecognizer {
        let recognizer = UILongPressGestureRecognizer()
        recognizer.minimumPressDuration = 0.045
        recognizer.allowableMovement = 8
        recognizer.cancelsTouchesInView = false
        recognizer.delaysTouchesBegan = false
        recognizer.delaysTouchesEnded = false
        recognizer.delegate = context.coordinator
        return recognizer
    }

    func updateUIGestureRecognizer(
        _ recognizer: UILongPressGestureRecognizer,
        context: Context
    ) {
        context.coordinator.onRecognized = onRecognized
    }

    func handleUIGestureRecognizerAction(
        _ recognizer: UILongPressGestureRecognizer,
        context: Context
    ) {
        guard recognizer.state == .began else { return }
        context.coordinator.onRecognized()
    }
}

private struct GaryxThreadActionMenuHostModifier: ViewModifier {
    let bottomInset: CGFloat

    @Environment(\.garyxMotion) private var motion
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @ScaledMetric(relativeTo: .body) private var rowHeight: CGFloat = GaryxThreadActionMenuMetrics.rowHeight
    @ScaledMetric(relativeTo: .body) private var verticalPadding: CGFloat = GaryxThreadActionMenuMetrics.verticalPadding

    func body(content: Content) -> some View {
        content.overlayPreferenceValue(GaryxThreadActionMenuPreferenceKey.self) { request in
            GeometryReader { proxy in
                let panelWidth = resolvedPanelWidth(in: proxy)
                ZStack(alignment: .topLeading) {
                    if let request {
                        Color.clear
                            .contentShape(Rectangle())
                            .onTapGesture { request.dismiss() }

                        focusScrim(request: request, proxy: proxy)
                            .allowsHitTesting(false)

                        GaryxThreadActionMenuPanel(request: request)
                            .frame(width: panelWidth)
                            .offset(panelOffset(for: request, panelWidth: panelWidth, in: proxy))
                            .garyxMaterializeTransition(
                                .threadMenu,
                                anchor: .bottomLeading
                            )
                    }
                }
                .animation(motion.animation(.threadMenu), value: request?.token)
            }
            .ignoresSafeArea()
        }
    }

    private func focusScrim(
        request: GaryxThreadActionMenuRequest,
        proxy: GeometryProxy
    ) -> some View {
        let sourceRect = focusedSourceRect(for: request, in: proxy)
        return Path { path in
            path.addRect(CGRect(origin: .zero, size: proxy.size))
            path.addRoundedRect(
                in: sourceRect,
                cornerSize: CGSize(
                    width: GaryxThreadActionMenuMetrics.sourceCornerRadius,
                    height: GaryxThreadActionMenuMetrics.sourceCornerRadius
                )
            )
        }
        .fill(Color.black.opacity(0.14), style: FillStyle(eoFill: true))
    }

    private func focusedSourceRect(
        for request: GaryxThreadActionMenuRequest,
        in proxy: GeometryProxy
    ) -> CGRect {
        proxy[request.anchor].insetBy(
            dx: GaryxThreadActionMenuMetrics.sourceHorizontalInset,
            dy: GaryxThreadActionMenuMetrics.sourceVerticalInset
        )
    }

    private func panelOffset(
        for request: GaryxThreadActionMenuRequest,
        panelWidth: CGFloat,
        in proxy: GeometryProxy
    ) -> CGSize {
        let sourceRect = focusedSourceRect(for: request, in: proxy)
        let panelHeight = CGFloat(request.items.count) * rowHeight + verticalPadding * 2
        let margin = GaryxThreadActionMenuMetrics.surfaceMargin
        let x = min(
            max(sourceRect.minX, margin),
            max(margin, proxy.size.width - panelWidth - margin)
        )
        let aboveY = sourceRect.minY - panelHeight - GaryxThreadActionMenuMetrics.panelGap
        let belowY = sourceRect.maxY + GaryxThreadActionMenuMetrics.panelGap
        let availableBottom = proxy.size.height - bottomInset - margin
        let y: CGFloat
        if aboveY >= margin {
            y = aboveY
        } else {
            y = min(belowY, max(margin, availableBottom - panelHeight))
        }
        return CGSize(width: x, height: y)
    }

    private func resolvedPanelWidth(in proxy: GeometryProxy) -> CGFloat {
        if dynamicTypeSize.garyxUsesExpandedReadingLayout {
            return min(340, proxy.size.width - GaryxThreadActionMenuMetrics.surfaceMargin * 2)
        }
        return min(
            max(
                proxy.size.width * GaryxThreadActionMenuMetrics.menuWidthFraction,
                GaryxThreadActionMenuMetrics.minimumMenuWidth
            ),
            GaryxThreadActionMenuMetrics.maximumMenuWidth
        )
    }

}

private struct GaryxThreadActionMenuPanel: View {
    @ScaledMetric(relativeTo: .body) private var rowHeight: CGFloat = GaryxThreadActionMenuMetrics.rowHeight
    @ScaledMetric(relativeTo: .body) private var verticalPadding: CGFloat = GaryxThreadActionMenuMetrics.verticalPadding
    let request: GaryxThreadActionMenuRequest

    var body: some View {
        GaryxGlassPanel(
            cornerRadius: GaryxThreadActionMenuMetrics.cornerRadius,
            tint: Color(.systemBackground).opacity(0.90),
            shadowOpacity: 0.15
        ) {
            VStack(spacing: 0) {
                ForEach(request.items) { item in
                    Button {
                        request.dismiss()
                        item.handler()
                    } label: {
                        HStack(spacing: 10) {
                            Image(systemName: item.systemImage)
                                .font(GaryxFont.fixedSystem(size: 18, weight: .regular))
                                .frame(width: 28)

                            Text(item.title)
                                .font(GaryxFont.body())
                                .garyxReadingLineLimit()

                            Spacer(minLength: 0)
                        }
                        .foregroundStyle(
                            item.role == .destructive ? GaryxTheme.danger : Color.primary
                        )
                        .padding(.horizontal, 18)
                        .frame(minHeight: rowHeight)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(GaryxThreadActionMenuButtonStyle())
                    .disabled(!item.isEnabled)
                    .opacity(item.isEnabled ? 1 : 0.42)
                }
            }
            .padding(.vertical, verticalPadding)
        }
    }
}

private struct GaryxThreadActionMenuButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .background(Color.primary.opacity(configuration.isPressed ? 0.055 : 0))
    }
}

/// Plain share-sheet host for menu actions that need `UIActivityViewController`
/// (the in-place menu cannot embed `ShareLink`).
struct GaryxActivityShareSheet: UIViewControllerRepresentable {
    let items: [Any]

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: items, applicationActivities: nil)
    }

    func updateUIViewController(_ controller: UIActivityViewController, context: Context) {}
}
