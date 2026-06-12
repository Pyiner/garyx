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
    func garyxMessageMenuHost(bottomInset: CGFloat = 0) -> some View {
        modifier(GaryxMessageMenuHostModifier(bottomInset: bottomInset))
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
                        UIImpactFeedbackGenerator(style: .light).impactOccurred()
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

    private static let menuWidth: CGFloat = 236
    private static let rowHeight: CGFloat = 46
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
                            .frame(width: Self.menuWidth)
                            .offset(panelOffset(for: request, in: proxy))
                            .transition(.opacity.combined(with: .scale(scale: 0.97)))
                    }
                }
                .animation(.easeOut(duration: 0.14), value: request?.token)
            }
        }
    }

    private func panelOffset(for request: GaryxMessageMenuRequest, in proxy: GeometryProxy) -> CGSize {
        let rect = proxy[request.anchor]
        let size = proxy.size
        let menuHeight = CGFloat(request.items.count) * Self.rowHeight + 12

        var x: CGFloat
        switch request.edge {
        case .leading: x = rect.minX
        case .trailing: x = rect.maxX - Self.menuWidth
        }
        x = min(max(x, Self.margin), max(Self.margin, size.width - Self.menuWidth - Self.margin))

        let bottomLimit = size.height - bottomInset - Self.margin
        var y = rect.maxY + 8
        if y + menuHeight > bottomLimit {
            y = rect.minY - menuHeight - 8
        }
        y = min(max(y, Self.margin), max(Self.margin, bottomLimit - menuHeight))
        return CGSize(width: x, height: y)
    }
}

private struct GaryxMessageMenuPanel: View {
    let request: GaryxMessageMenuRequest

    var body: some View {
        GaryxGlassPanel(cornerRadius: 18, fallbackMaterial: .regularMaterial, shadowOpacity: 0.16) {
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
                                .font(.system(size: 15, weight: .regular))
                                .foregroundStyle(.secondary)
                        }
                        .padding(.horizontal, 14)
                        .frame(height: 46)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.vertical, 6)
        }
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
