import Foundation
import SwiftUI
import UIKit

enum GaryxTheme {
    private static let sampledLightBackground = UIColor(
        red: 253.0 / 255.0,
        green: 253.0 / 255.0,
        blue: 253.0 / 255.0,
        alpha: 1
    )
    private static let adaptivePageBackground = UIColor { traits in
        traits.userInterfaceStyle == .dark ? .systemBackground : sampledLightBackground
    }
    private static let adaptiveControlTint = UIColor { traits in
        traits.userInterfaceStyle == .dark ? .systemGray2 : .label
    }

    static let background = Color(adaptivePageBackground)
    static let sidebar = Color(adaptivePageBackground)
    static let header = Color(adaptivePageBackground)
    static let surface = Color(adaptivePageBackground)
    static let input = Color(.secondarySystemGroupedBackground)
    static let primaryText = Color.primary
    static let secondaryText = Color.secondary
    static let tertiaryText = Color(.tertiaryLabel)
    /// Monochrome tint for ordinary controls. It is black in light appearance
    /// and a mid-gray in dark appearance so a switch's white thumb stays clear.
    static let controlTint = Color(adaptiveControlTint)
    static let accent = Color(red: 0.000, green: 0.635, blue: 0.250)
    /// Inline links in rendered content. The system link blue adapts to
    /// light/dark; the green accent stays reserved for running/success
    /// semantics and never colors links.
    static let link = Color(uiColor: .link)
    static let warning = Color.orange
    static let danger = Color.red
    static let hairline = Color.primary.opacity(0.08)
    /// Capsule-favorite semantic colors. Gold is intentionally reserved for
    /// this state and must not be reused as a general accent.
    static let capsuleFavoriteGoldTop = Color(
        red: 255.0 / 255.0,
        green: 224.0 / 255.0,
        blue: 130.0 / 255.0
    )
    static let capsuleFavoriteGoldBottom = Color(
        red: 245.0 / 255.0,
        green: 166.0 / 255.0,
        blue: 35.0 / 255.0
    )
    static let capsuleFavoriteGlow = Color(
        red: 245.0 / 255.0,
        green: 166.0 / 255.0,
        blue: 35.0 / 255.0
    )
}

/// Shared Capsule favorite glyph for gallery badges and focused chrome.
struct GaryxFavoriteStar: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.garyxMotion) private var motion

    let isFavorited: Bool
    var size: CGFloat = 18

    var body: some View {
        star
            .animation(motion.animation(.favoriteToggle), value: isFavorited)
            .shadow(
                color: GaryxTheme.capsuleFavoriteGlow.opacity(
                    isFavorited ? (colorScheme == .dark ? 0.30 : 0.45) : 0
                ),
                radius: 4
            )
    }

    private var star: some View {
        Image(systemName: isFavorited ? "star.fill" : "star")
            .font(GaryxFont.fixedSystem(size: size, weight: .semibold))
            .foregroundStyle(
                isFavorited
                    ? AnyShapeStyle(
                        LinearGradient(
                            colors: [
                                GaryxTheme.capsuleFavoriteGoldTop,
                                GaryxTheme.capsuleFavoriteGoldBottom,
                            ],
                            startPoint: .top,
                            endPoint: .bottom
                        )
                    )
                    : AnyShapeStyle(Color.secondary)
            )
    }
}

enum GaryxSafeAreaChrome {
    static let pageBackgroundEdges: Edge.Set = .all

    static func installWindowDefaults() {
        UIWindow.appearance().backgroundColor = .clear
    }
}

enum GaryxFont {
    static func largeTitle(weight: Font.Weight = .regular) -> Font {
        font(.largeTitle, weight: weight)
    }

    static func title(weight: Font.Weight = .regular) -> Font {
        font(.title, weight: weight)
    }

    static func title2(weight: Font.Weight = .regular) -> Font {
        font(.title2, weight: weight)
    }

    static func title3(weight: Font.Weight = .regular) -> Font {
        font(.title3, weight: weight)
    }

    static func headline(weight: Font.Weight = .semibold) -> Font {
        font(.headline, weight: weight)
    }

    static func body(weight: Font.Weight = .regular) -> Font {
        font(.body, weight: weight)
    }

    static func callout(weight: Font.Weight = .regular) -> Font {
        font(.callout, weight: weight)
    }

    static func subheadline(weight: Font.Weight = .regular) -> Font {
        font(.subheadline, weight: weight)
    }

    static func footnote(weight: Font.Weight = .regular) -> Font {
        font(.footnote, weight: weight)
    }

    static func caption(weight: Font.Weight = .regular) -> Font {
        font(.caption, weight: weight)
    }

    static func caption2(weight: Font.Weight = .regular) -> Font {
        font(.caption2, weight: weight)
    }

    static func monospaced(
        _ role: GaryxTypographyRole,
        weight: Font.Weight = .regular
    ) -> Font {
        font(role, weight: weight, design: .monospaced)
    }

    static func uiFont(
        _ role: GaryxTypographyRole,
        compatibleWith traitCollection: UITraitCollection? = nil
    ) -> UIFont {
        UIFont.preferredFont(
            forTextStyle: role.specification.textStyle.uiKitTextStyle,
            compatibleWith: traitCollection
        )
    }

    /// Fixed point sizes are reserved for SF Symbols, avatar initials, and
    /// explicitly documented fixed chrome geometry. Readable text must use a
    /// semantic role above so Dynamic Type and the system optical tables apply.
    static func fixedSystem(size: CGFloat, weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight)
    }

    private static func font(
        _ role: GaryxTypographyRole,
        weight: Font.Weight,
        design: Font.Design = .default
    ) -> Font {
        .system(role.specification.textStyle.swiftUITextStyle, design: design, weight: weight)
    }
}

private extension GaryxTypographyTextStyle {
    var swiftUITextStyle: Font.TextStyle {
        switch self {
        case .largeTitle: .largeTitle
        case .title: .title
        case .title2: .title2
        case .title3: .title3
        case .headline: .headline
        case .body: .body
        case .callout: .callout
        case .subheadline: .subheadline
        case .footnote: .footnote
        case .caption: .caption
        case .caption2: .caption2
        }
    }

    var uiKitTextStyle: UIFont.TextStyle {
        switch self {
        case .largeTitle: .largeTitle
        case .title: .title1
        case .title2: .title2
        case .title3: .title3
        case .headline: .headline
        case .body: .body
        case .callout: .callout
        case .subheadline: .subheadline
        case .footnote: .footnote
        case .caption: .caption1
        case .caption2: .caption2
        }
    }
}

private struct GaryxTypographyScaleBoundaryModifier: ViewModifier {
    let boundary: GaryxTypographyScaleBoundary

    @ViewBuilder
    func body(content: Content) -> some View {
        switch boundary.maximumCategory {
        case nil:
            content
        case .extraExtraLarge:
            content.dynamicTypeSize(...DynamicTypeSize.xxLarge)
        }
    }
}

private struct GaryxReadingLineLimitModifier: ViewModifier {
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    let compactLimit: Int?

    func body(content: Content) -> some View {
        content.lineLimit(dynamicTypeSize.garyxUsesExpandedReadingLayout ? nil : compactLimit)
    }
}

extension DynamicTypeSize {
    /// The largest standard category already needs the same wrapping freedom
    /// as accessibility categories on compact iPhone rows.
    var garyxUsesExpandedReadingLayout: Bool {
        self >= .xxxLarge
    }
}

struct GaryxPrimaryCompactButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(GaryxFont.footnote(weight: .semibold))
            .foregroundStyle(Color(.systemBackground))
            .padding(.vertical, 6)
            .padding(.horizontal, 9)
            .frame(minHeight: 44)
            .background(Color(.label).opacity(configuration.isPressed ? 0.72 : 1), in: Capsule())
    }
}

struct GaryxPrimaryWideButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(GaryxFont.callout(weight: .semibold))
            .foregroundStyle(Color(.systemBackground))
            .padding(.horizontal, 16)
            .frame(minHeight: 46)
            .background(Color(.label).opacity(configuration.isPressed ? 0.72 : 1), in: Capsule())
            .opacity(configuration.isPressed ? 0.92 : 1)
    }
}

struct GaryxSecondaryButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(GaryxFont.footnote(weight: .semibold))
            .foregroundStyle(.primary)
            .padding(.vertical, 6)
            .padding(.horizontal, 9)
            .frame(minHeight: 44)
            .garyxAdaptiveGlass(.regular, isInteractive: true, in: Capsule())
            .opacity(configuration.isPressed ? 0.72 : 1)
    }
}

struct GaryxMiniIconButtonStyle: ButtonStyle {
    var isPrimary = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(GaryxFont.fixedSystem(size: 13, weight: .semibold))
            .foregroundStyle(isPrimary ? Color(.systemBackground) : Color.primary)
            .frame(width: 44, height: 44)
            .background(
                isPrimary
                    ? Color(.label).opacity(configuration.isPressed ? 0.72 : 1)
                    : Color.primary.opacity(configuration.isPressed ? 0.07 : 0),
                in: RoundedRectangle(cornerRadius: 7, style: .continuous)
            )
    }
}

struct GaryxIconButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(GaryxFont.fixedSystem(size: 15, weight: .semibold))
            .foregroundStyle(.primary)
            .frame(width: 44, height: 44)
            .garyxAdaptiveGlass(.regular, isInteractive: true, in: Circle())
            .opacity(configuration.isPressed ? 0.72 : 1)
    }
}

struct GaryxStopButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(.white)
            .frame(width: 32, height: 32)
            .background(GaryxTheme.danger.opacity(configuration.isPressed ? 0.72 : 1), in: Circle())
    }
}

private struct GaryxAdaptiveGlassModifier<S: Shape>: ViewModifier {
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    let style: GaryxAdaptiveGlassStyle
    let isInteractive: Bool
    let tint: Color?
    let shape: S
    let isEnabled: Bool

    @ViewBuilder
    func body(content: Content) -> some View {
        if reduceTransparency {
            opaqueFallback(content: content)
        } else {
            switch style {
            case .regular:
                content.glassEffect(resolvedGlass, in: shape)
            }
        }
    }

    @ViewBuilder
    private func opaqueFallback(content: Content) -> some View {
        if isEnabled {
            content.background {
                shape
                    .fill(Color(uiColor: .secondarySystemBackground))
                    .overlay {
                        if let tint {
                            shape.fill(tint)
                        }
                    }
            }
        } else {
            content
        }
    }

    private var resolvedGlass: Glass {
        guard isEnabled else { return .identity }
        var glass = Glass.regular
        if let tint {
            glass = glass.tint(tint)
        }
        if isInteractive {
            glass = glass.interactive()
        }
        return glass
    }
}

enum GaryxAdaptiveGlassStyle {
    case regular
}

struct GaryxAdaptiveGlassContainer<Content: View>: View {
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    let spacing: CGFloat
    private let content: () -> Content

    init(spacing: CGFloat, @ViewBuilder content: @escaping () -> Content) {
        self.spacing = spacing
        self.content = content
    }

    var body: some View {
        if reduceTransparency {
            content()
        } else {
            GlassEffectContainer(spacing: spacing) {
                content()
            }
        }
    }
}

private struct GaryxSoftScrollEdgeModifier: ViewModifier {
    let edges: Edge.Set

    func body(content: Content) -> some View {
        content.scrollEdgeEffectStyle(.soft, for: edges)
    }
}

private struct GaryxFloatingBottomChromeHeightKey: PreferenceKey {
    static var defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = max(value, nextValue())
    }
}

private struct GaryxFloatingBottomChromeModifier<Chrome: View>: ViewModifier {
    let onHeightChange: ((CGFloat) -> Void)?
    let chrome: () -> Chrome

    func body(content: Content) -> some View {
        content
            .safeAreaInset(edge: .bottom, spacing: 0) {
                chrome()
                    .frame(maxWidth: .infinity)
                    .background(Color.clear)
                    .background {
                        GeometryReader { chromeGeometry in
                            Color.clear.preference(
                                key: GaryxFloatingBottomChromeHeightKey.self,
                                value: chromeGeometry.size.height
                            )
                        }
                    }
                    .ignoresSafeArea(.container, edges: .bottom)
            }
            .onPreferenceChange(GaryxFloatingBottomChromeHeightKey.self) { height in
                onHeightChange?(height)
            }
    }
}

extension View {
    /// Applies one of the centrally documented fixed-geometry exceptions.
    /// Reading surfaces use `.readingSurface`, which intentionally has no cap.
    func garyxTypographyBoundary(_ boundary: GaryxTypographyScaleBoundary) -> some View {
        modifier(GaryxTypographyScaleBoundaryModifier(boundary: boundary))
    }

    /// Compact rows may stay concise below the largest standard size. At
    /// XXXL and accessibility sizes they expose complete text instead of
    /// preserving a one-line desktop-like density.
    func garyxReadingLineLimit(_ compactLimit: Int? = 1) -> some View {
        modifier(GaryxReadingLineLimitModifier(compactLimit: compactLimit))
    }

    /// Pins vertical scroll content to the scroll container's width so stray
    /// horizontal child overflow can never widen the scroll content and make
    /// the page horizontally pannable. Apply to the top-level content of a
    /// vertical `ScrollView`.
    func garyxVerticalScrollContentWidth(
        maxWidth: CGFloat = .infinity,
        alignment: Alignment = .top
    ) -> some View {
        containerRelativeFrame(.horizontal, alignment: alignment) { length, _ in
            min(length, maxWidth)
        }
    }

    func garyxRootChromeBackground() -> some View {
        background(GaryxHostingBackgroundClearer())
    }

    func garyxPageBackground() -> some View {
        background(GaryxTheme.background.ignoresSafeArea(edges: GaryxSafeAreaChrome.pageBackgroundEdges))
    }

    func garyxFloatingBottomChrome<Chrome: View>(
        onHeightChange: ((CGFloat) -> Void)? = nil,
        @ViewBuilder _ chrome: @escaping () -> Chrome
    ) -> some View {
        modifier(GaryxFloatingBottomChromeModifier(onHeightChange: onHeightChange, chrome: chrome))
    }

    func garyxInputStyle() -> some View {
        self
            .font(GaryxFont.body())
            .foregroundStyle(.primary)
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .background(GaryxTheme.input, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .stroke(GaryxTheme.hairline, lineWidth: 1)
            }
    }

    func garyxCardStyle() -> some View {
        self
            .padding(8)
            .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .stroke(GaryxTheme.hairline, lineWidth: 1)
            }
    }

    func garyxAdaptiveGlass(
        _ style: GaryxAdaptiveGlassStyle,
        isInteractive: Bool,
        tint: Color? = nil,
        in shape: some Shape,
        isEnabled: Bool = true
    ) -> some View {
        modifier(
            GaryxAdaptiveGlassModifier(
                style: style,
                isInteractive: isInteractive,
                tint: tint,
                shape: shape,
                isEnabled: isEnabled
            )
        )
    }

    func garyxAdaptiveSoftScrollEdge(for edges: Edge.Set) -> some View {
        modifier(GaryxSoftScrollEdgeModifier(edges: edges))
    }

    @ViewBuilder
    func garyxAdaptiveTopBar<Bar: View>(@ViewBuilder _ bar: () -> Bar) -> some View {
        self.safeAreaInset(edge: .top, spacing: 0, content: bar)
    }

    @ViewBuilder
    func `if`<Content: View>(_ condition: Bool, transform: (Self) -> Content) -> some View {
        if condition {
            transform(self)
        } else {
            self
        }
    }
}

private struct GaryxHostingBackgroundClearer: UIViewRepresentable {
    func makeUIView(context: Context) -> UIView {
        let view = UIView(frame: .zero)
        clearHostingBackground(from: view)
        return view
    }

    func updateUIView(_ uiView: UIView, context: Context) {
        clearHostingBackground(from: uiView)
    }

    private func clearHostingBackground(from view: UIView) {
        DispatchQueue.main.async {
            // SwiftUI hosting views otherwise provide an opaque system background
            // behind safe-area gaps, which makes shared bottom chrome appear as a
            // white base even when its own background is clear.
            view.backgroundColor = .clear
            view.window?.backgroundColor = .clear

            var ancestor = view.superview
            while let current = ancestor {
                current.backgroundColor = .clear
                ancestor = current.superview
            }
        }
    }
}

// Relative-time formatting and ISO8601 parsing moved to GaryxMobileCore
// (`GaryxRelativeTimestamp.swift`) so `swift test` exercises the production
// implementation. In the app target those Core sources compile into this same
// module, so call sites here use them without importing GaryxMobileCore.
