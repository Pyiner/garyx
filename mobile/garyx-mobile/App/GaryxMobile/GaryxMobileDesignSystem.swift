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

    static let background = Color(adaptivePageBackground)
    static let sidebar = Color(adaptivePageBackground)
    static let header = Color(adaptivePageBackground)
    static let surface = Color(adaptivePageBackground)
    static let input = Color(.secondarySystemGroupedBackground)
    static let primaryText = Color.primary
    static let secondaryText = Color.secondary
    static let tertiaryText = Color(.tertiaryLabel)
    static let accent = Color(red: 0.000, green: 0.635, blue: 0.250)
    /// Inline links in rendered content. The system link blue adapts to
    /// light/dark; the green accent stays reserved for running/success
    /// semantics and never colors links.
    static let link = Color(uiColor: .link)
    static let warning = Color.orange
    static let danger = Color.red
    static let hairline = Color.primary.opacity(0.08)
}

enum GaryxSafeAreaChrome {
    static let pageBackgroundEdges: Edge.Set = .all

    static func installWindowDefaults() {
        UIWindow.appearance().backgroundColor = .clear
    }
}

enum GaryxFont {
    static func largeTitle(weight: Font.Weight = .regular) -> Font {
        .system(size: 34, weight: weight)
    }

    static func title2(weight: Font.Weight = .regular) -> Font {
        .system(size: 22, weight: weight)
    }

    static func title3(weight: Font.Weight = .regular) -> Font {
        .system(size: 20, weight: weight)
    }

    static func body(weight: Font.Weight = .regular) -> Font {
        .system(size: 17, weight: weight)
    }

    static func callout(weight: Font.Weight = .regular) -> Font {
        .system(size: 16, weight: weight)
    }

    static func subheadline(weight: Font.Weight = .regular) -> Font {
        .system(size: 15, weight: weight)
    }

    static func footnote(weight: Font.Weight = .regular) -> Font {
        .system(size: 13, weight: weight)
    }

    static func caption(weight: Font.Weight = .regular) -> Font {
        .system(size: 12, weight: weight)
    }

    static func system(size: CGFloat, weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight)
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
            .garyxAdaptiveGlass(.regular, isInteractive: true, fallbackMaterial: .thinMaterial, in: Capsule())
            .opacity(configuration.isPressed ? 0.72 : 1)
    }
}

struct GaryxMiniIconButtonStyle: ButtonStyle {
    var isPrimary = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(GaryxFont.system(size: 13, weight: .semibold))
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
            .font(GaryxFont.system(size: 15, weight: .semibold))
            .foregroundStyle(.primary)
            .frame(width: 44, height: 44)
            .garyxAdaptiveGlass(.regular, isInteractive: true, fallbackMaterial: .thinMaterial, in: Circle())
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
    let style: GaryxAdaptiveGlassStyle
    let isInteractive: Bool
    let tint: Color?
    let fallbackMaterial: Material
    let shape: S

    @ViewBuilder
    func body(content: Content) -> some View {
#if compiler(>=6.2)
        if #available(iOS 26, *) {
            switch style {
            case .automatic:
                content.glassEffect(in: shape)
            case .regular:
                content.glassEffect(resolvedGlass, in: shape)
            }
        } else {
            fallback(content: content)
        }
#else
        fallback(content: content)
#endif
    }

    @ViewBuilder
    private func fallback(content: Content) -> some View {
        if let tint {
            content.background(tint, in: shape)
        } else {
            content.background(fallbackMaterial, in: shape)
        }
    }

#if compiler(>=6.2)
    @available(iOS 26, *)
    private var resolvedGlass: Glass {
        var glass = Glass.regular
        if let tint {
            glass = glass.tint(tint)
        }
        if isInteractive {
            glass = glass.interactive()
        }
        return glass
    }
#endif
}

enum GaryxAdaptiveGlassStyle {
    case automatic
    case regular
}

struct GaryxAdaptiveGlassContainer<Content: View>: View {
    let spacing: CGFloat
    private let content: () -> Content

    init(spacing: CGFloat, @ViewBuilder content: @escaping () -> Content) {
        self.spacing = spacing
        self.content = content
    }

    var body: some View {
#if compiler(>=6.2)
        if #available(iOS 26, *) {
            GlassEffectContainer(spacing: spacing) {
                content()
            }
        } else {
            content()
        }
#else
        content()
#endif
    }
}

private struct GaryxSoftScrollEdgeModifier: ViewModifier {
    let edges: Edge.Set

    func body(content: Content) -> some View {
#if compiler(>=6.2)
        if #available(iOS 26, *) {
            content.scrollEdgeEffectStyle(.soft, for: edges)
        } else {
            content
        }
#else
        content
#endif
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

    func garyxAdaptiveGlass(_ style: GaryxAdaptiveGlassStyle, in shape: some Shape) -> some View {
        garyxAdaptiveGlass(style, isInteractive: false, tint: nil, fallbackMaterial: .thinMaterial, in: shape)
    }

    func garyxAdaptiveGlass(
        _ style: GaryxAdaptiveGlassStyle,
        isInteractive: Bool,
        tint: Color? = nil,
        fallbackMaterial: Material = .thinMaterial,
        in shape: some Shape
    ) -> some View {
        modifier(
            GaryxAdaptiveGlassModifier(
                style: style,
                isInteractive: isInteractive,
                tint: tint,
                fallbackMaterial: fallbackMaterial,
                shape: shape
            )
        )
    }

    func garyxAdaptiveGlass(in shape: some Shape) -> some View {
        garyxAdaptiveGlass(.automatic, isInteractive: false, tint: nil, fallbackMaterial: .thinMaterial, in: shape)
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

func garyxFormattedTaskTimestamp(_ value: String?) -> String {
    guard let value, let date = garyxISO8601Date(from: value) else {
        return ""
    }
    let diff = max(0, Date().timeIntervalSince(date))
    let minutes = Int(diff / 60)
    let hours = Int(diff / 3_600)
    let days = Int(diff / 86_400)
    let months = days / 30
    if minutes < 1 { return "now" }
    if minutes < 60 { return "\(minutes)m" }
    if hours < 24 { return "\(hours)h" }
    if days < 30 { return "\(days)d" }
    if months < 12 { return "\(months)mo" }
    return "\(days / 365)y"
}

func garyxThreadDate(from value: String) -> Date? {
    garyxISO8601Date(from: value)
}

// Formatter construction is expensive and these run per row per render in
// sidebar/task lists, so keep shared instances (ISO8601DateFormatter is
// thread-safe) plus a bounded parse cache keyed by the raw timestamp string.
private let garyxISO8601FractionalFormatter: ISO8601DateFormatter = {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return formatter
}()

private let garyxISO8601StandardFormatter: ISO8601DateFormatter = {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime]
    return formatter
}()

private let garyxISO8601DateCache: NSCache<NSString, NSDate> = {
    let cache = NSCache<NSString, NSDate>()
    cache.countLimit = 4096
    return cache
}()

private func garyxISO8601Date(from value: String) -> Date? {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return nil }

    let cacheKey = trimmed as NSString
    if let cached = garyxISO8601DateCache.object(forKey: cacheKey) {
        return cached as Date
    }
    let parsed = garyxISO8601FractionalFormatter.date(from: trimmed)
        ?? garyxISO8601StandardFormatter.date(from: trimmed)
    if let parsed {
        garyxISO8601DateCache.setObject(parsed as NSDate, forKey: cacheKey)
    }
    return parsed
}
