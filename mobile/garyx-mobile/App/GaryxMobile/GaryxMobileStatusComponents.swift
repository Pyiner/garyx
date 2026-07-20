import Foundation
import SwiftUI
import UIKit

struct GaryxStatusPill: View {
    enum Tone: Equatable {
        case good
        case warning
        case danger
        case muted
    }

    let text: String
    let tone: Tone

    var body: some View {
        Text(text)
            .font(GaryxFont.caption2(weight: .semibold))
            .foregroundStyle(color)
            .garyxReadingLineLimit()
            .fixedSize(horizontal: true, vertical: false)
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(color.opacity(0.10), in: Capsule())
            // Status pills must stay inline with their parent row; XXL avoids
            // an unbounded badge taking over the row while still scaling it.
            .garyxTypographyBoundary(.compactBadgeChrome)
    }

    private var color: Color {
        switch tone {
        case .good:
            GaryxTheme.accent
        case .warning:
            GaryxTheme.warning
        case .danger:
            GaryxTheme.danger
        case .muted:
            .secondary
        }
    }
}

struct GaryxGlobalErrorToastHost: View {
    @Environment(GaryxHomeObservationStore.self) private var homeObservationStore
    @Environment(\.garyxMotion) private var motion
    let topOffset: CGFloat
    let onClearError: (String) -> Void

    @State private var visibleError: String?
    @State private var toastToken = 0

    var body: some View {
        Group {
            if let visibleError {
                GaryxGlobalErrorToast(text: visibleError) {
                    hide(message: visibleError)
                }
                .padding(.horizontal, 18)
                .padding(.top, topOffset)
                .garyxMaterializeTransition(.toast, anchor: .top)
                .zIndex(100)
            }
        }
        .frame(maxWidth: .infinity, alignment: .top)
        .onAppear {
            present(homeObservationStore.lastError)
        }
        .onChange(of: homeObservationStore.lastError) { _, newValue in
            present(newValue)
        }
        .task(id: toastToken) {
            guard let message = visibleError else { return }
            try? await Task.sleep(nanoseconds: 3_200_000_000)
            guard !Task.isCancelled else { return }
            await MainActor.run {
                hide(message: message)
            }
        }
    }

    private func present(_ message: String?) {
        let trimmed = message?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !trimmed.isEmpty else {
            toastToken += 1
            withAnimation(motion.animation(.toast)) {
                visibleError = nil
            }
            return
        }

        toastToken += 1
        withAnimation(motion.animation(.toast)) {
            visibleError = trimmed
        }
    }

    private func hide(message: String) {
        guard visibleError == message else { return }
        toastToken += 1
        withAnimation(motion.animation(.toast)) {
            visibleError = nil
        }
        onClearError(message)
    }
}

struct GaryxGlobalErrorToast: View {
    @ScaledMetric(relativeTo: .footnote) private var verticalPadding: CGFloat = 10
    @ScaledMetric(relativeTo: .footnote) private var contentSpacing: CGFloat = 9
    let text: String
    let onDismiss: () -> Void

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 14, style: .continuous)

        Button(action: onDismiss) {
            HStack(spacing: contentSpacing) {
                Image(systemName: "exclamationmark.circle.fill")
                    .font(GaryxFont.fixedSystem(size: 14, weight: .semibold))
                    .foregroundStyle(GaryxTheme.danger.opacity(0.86))

                Text(text)
                    .font(GaryxFont.footnote(weight: .medium))
                    .foregroundStyle(.primary)
                    .garyxReadingLineLimit(2)
                    .multilineTextAlignment(.leading)
                    .fixedSize(horizontal: false, vertical: true)

                Spacer(minLength: 2)

                Image(systemName: "xmark")
                    .font(GaryxFont.fixedSystem(size: 10, weight: .bold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, verticalPadding)
            .frame(maxWidth: 360, alignment: .leading)
            .garyxAdaptiveGlass(
                .regular,
                isInteractive: true,
                in: shape
            )
            .contentShape(shape)
            .overlay {
                shape.stroke(GaryxTheme.hairline, lineWidth: 1)
            }
            .shadow(color: Color.black.opacity(0.10), radius: 18, y: 8)
        }
        .buttonStyle(GaryxPressableRowStyle())
        .accessibilityLabel(text)
        .accessibilityHint("Dismiss")
    }
}

struct GaryxEmptyPanelView: View {
    let icon: String
    let title: String
    let text: String

    var body: some View {
        GaryxStateView(
            style: .panel,
            state: .empty(icon: icon),
            title: title,
            text: text
        )
    }
}

struct GaryxLoadingPanelView: View {
    let title: String

    var body: some View {
        GaryxStateView(
            style: .panel,
            state: .loading,
            title: title
        )
    }
}

struct GaryxStateView: View {
    enum Style {
        case panel
        case inline
    }

    enum State {
        case loading
        case empty(icon: String)
    }

    let style: Style
    let state: State
    let title: String
    var text = ""

    var body: some View {
        VStack(spacing: 12) {
            indicator
            Text(title)
                .font(titleFont)
                .foregroundStyle(titleColor)
                .multilineTextAlignment(.center)
            if !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Text(text)
                    .font(GaryxFont.callout())
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 24)
        .padding(.vertical, verticalPadding)
    }

    @ViewBuilder
    private var indicator: some View {
        switch state {
        case .loading:
            ProgressView()
                .controlSize(.regular)
        case .empty(let icon):
            if icon == GaryxMobilePanel.capsules.iconName {
                // Capsules empty state uses the gem glyph in the accent gradient.
                GaryxCapsuleGlyph(style: .accent)
                    .frame(width: emptyGlyphSize, height: emptyGlyphSize)
            } else {
                Image(systemName: icon)
                    .font(iconFont)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var emptyGlyphSize: CGFloat {
        switch style {
        case .panel: return 40
        case .inline: return 30
        }
    }

    private var titleFont: Font {
        if isLoading {
            return GaryxFont.callout(weight: .medium)
        }
        switch style {
        case .panel:
            return GaryxFont.body(weight: .semibold)
        case .inline:
            return GaryxFont.callout(weight: .semibold)
        }
    }

    private var titleColor: Color {
        if isLoading {
            return .secondary
        }
        switch style {
        case .panel:
            return .primary
        case .inline:
            return .secondary
        }
    }

    private var iconFont: Font {
        switch style {
        case .panel:
            GaryxFont.title2(weight: .medium)
        case .inline:
            GaryxFont.fixedSystem(size: 28, weight: .medium)
        }
    }

    private var verticalPadding: CGFloat {
        switch style {
        case .panel:
            36
        case .inline:
            42
        }
    }

    private var isLoading: Bool {
        if case .loading = state {
            return true
        }
        return false
    }
}

struct GaryxInlineStateView: View {
    let title: String
    var icon: String?
    var isLoading = false

    var body: some View {
        GaryxStateView(
            style: .inline,
            state: isLoading ? .loading : .empty(icon: icon ?? "info.circle"),
            title: title
        )
    }
}

struct GaryxFieldLabel: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        Text(text)
            .font(GaryxFont.caption(weight: .semibold))
            .foregroundStyle(.secondary)
            .textCase(.uppercase)
    }
}

/// Single-direction "ink in water" loading comet: a tapering trail of dots that
/// sweeps clockwise, widest and most opaque at the head and dissolving into the
/// tail. Used for the thread toolbar loading state in place of the ellipsis.
struct GaryxInkSpinner: View {
    @Environment(\.garyxMotion) private var motion
    var size: CGFloat = 22
    var color: Color = .primary

    var body: some View {
        TimelineView(
            .animation(
                minimumInterval: GaryxMotion.timelineMinimumInterval,
                paused: motion.pausesContinuousMotion(.inkSpinner)
            )
        ) { context in
            let elapsed = context.date.timeIntervalSinceReferenceDate
            let period = motion.cycleDuration(.inkSpinner)
            let head = elapsed.truncatingRemainder(dividingBy: period) / period

            Canvas { ctx, canvasSize in
                let side = min(canvasSize.width, canvasSize.height)
                let headRadius = side * 0.12
                let ring = side / 2 - headRadius
                let center = CGPoint(x: canvasSize.width / 2, y: canvasSize.height / 2)
                let tailSpan = 0.82
                let segments = 70
                // Screen space has y pointing down, so an increasing angle
                // sweeps clockwise; the trail lags behind the head angle.
                let headAngle = head * 2 * .pi - .pi / 2

                for index in 0..<segments {
                    let f = Double(index) / Double(segments - 1)
                    let angle = headAngle - f * tailSpan * 2 * .pi
                    let dotRadius = headRadius * (1 - f * 0.9)
                    let alpha = pow(1 - f, 1.3)
                    let point = CGPoint(
                        x: center.x + ring * cos(angle),
                        y: center.y + ring * sin(angle)
                    )
                    let rect = CGRect(
                        x: point.x - dotRadius,
                        y: point.y - dotRadius,
                        width: dotRadius * 2,
                        height: dotRadius * 2
                    )
                    ctx.fill(Path(ellipseIn: rect), with: .color(color.opacity(alpha)))
                }
            }
            .blur(radius: 0.3)
        }
        .frame(width: size, height: size)
    }
}

struct GaryxToolbarIcon: View {
    var systemName: String?
    var customContent: (() -> AnyView)?

    init(systemName: String) {
        self.systemName = systemName
        self.customContent = nil
    }

    init<Content: View>(@ViewBuilder content: @escaping () -> Content) {
        self.systemName = nil
        self.customContent = { AnyView(content()) }
    }

    var body: some View {
        Group {
            if let systemName {
                Image(systemName: systemName)
                    .font(GaryxFont.fixedSystem(size: 18, weight: .semibold))
                    .foregroundStyle(.primary)
            } else if let customContent {
                customContent()
            }
        }
        .frame(width: 44, height: 44)
        .garyxAdaptiveGlass(.regular, isInteractive: true, in: Circle())
        .contentShape(Rectangle())
    }
}

struct GaryxCompactGlassIcon: View {
    let systemName: String
    var diameter: CGFloat = 32
    var iconSize: CGFloat = 13

    var body: some View {
        Image(systemName: systemName)
            .font(GaryxFont.fixedSystem(size: iconSize, weight: .medium))
            .foregroundStyle(.primary)
            .frame(width: diameter, height: diameter)
            .garyxAdaptiveGlass(.regular, isInteractive: true, in: Circle())
            .contentShape(Rectangle())
    }
}

struct GaryxGlassPanel<Content: View>: View {
    var cornerRadius: CGFloat = 24
    var tint: Color? = Color(.systemBackground).opacity(0.96)
    var shadowOpacity: Double = 0.055
    private let content: () -> Content

    init(
        cornerRadius: CGFloat = 24,
        tint: Color? = Color(.systemBackground).opacity(0.96),
        shadowOpacity: Double = 0.055,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.cornerRadius = cornerRadius
        self.tint = tint
        self.shadowOpacity = shadowOpacity
        self.content = content
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)

        content()
            .garyxAdaptiveGlass(
                .regular,
                isInteractive: false,
                tint: tint,
                in: shape
            )
            .clipShape(shape)
            .overlay {
                shape
                    .stroke(Color.white.opacity(0.34), lineWidth: 0.7)
            }
            .overlay {
                shape
                    .stroke(Color.primary.opacity(0.055), lineWidth: 1)
            }
            .shadow(color: Color.black.opacity(shadowOpacity), radius: 24, x: 0, y: 10)
    }
}

struct GaryxGlassSearchField: View {
    @ScaledMetric(relativeTo: .subheadline) private var verticalPadding: CGFloat = 9
    let placeholder: String
    @Binding var text: String

    init(_ placeholder: String = "Search", text: Binding<String>) {
        self.placeholder = placeholder
        self._text = text
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 22, style: .continuous)

        HStack(spacing: 10) {
            Image(systemName: "magnifyingglass")
                .font(GaryxFont.fixedSystem(size: 15, weight: .medium))
                .foregroundStyle(.secondary)

            TextField(placeholder, text: $text)
                .font(GaryxFont.subheadline())
                .foregroundStyle(.primary)
                .textInputAutocapitalization(.never)
                .disableAutocorrection(true)

            if !text.isEmpty {
                Button {
                    text = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(GaryxFont.fixedSystem(size: 15, weight: .medium))
                        .foregroundStyle(.tertiary)
                }
                .buttonStyle(GaryxPressableRowStyle())
                .accessibilityLabel("Clear search")
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, verticalPadding)
        .frame(minHeight: 38)
        .garyxAdaptiveGlass(
            .regular,
            isInteractive: true,
            tint: Color(.systemBackground).opacity(0.92),
            in: shape
        )
        .overlay {
            shape
                .stroke(Color.white.opacity(0.34), lineWidth: 0.7)
        }
        .overlay {
            shape
                .stroke(Color.primary.opacity(0.055), lineWidth: 1)
        }
    }
}

struct GaryxSidebarMenuButton: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            GaryxHeaderMenuIcon()
                .frame(width: 48, height: 48)
                .contentShape(Rectangle())
        }
        .buttonStyle(GaryxPressableRowStyle(prepares: .drawerVisibilityCommitted))
        .accessibilityLabel("Open menu")
    }
}

struct GaryxHeaderMenuIcon: View {
    var body: some View {
        Image(systemName: "line.3.horizontal")
            .font(GaryxFont.fixedSystem(size: 17, weight: .semibold))
            .foregroundStyle(.primary)
            .frame(width: 44, height: 44)
            .garyxAdaptiveGlass(.regular, isInteractive: true, in: Circle())
            .contentShape(Rectangle())
    }
}

struct GaryxCircleBadge: View {
    let systemName: String
    let foreground: Color
    let background: Color
    var diameter: CGFloat = 32
    var iconSize: CGFloat = 12
    var iconWeight: Font.Weight = .bold

    var body: some View {
        Image(systemName: systemName)
            .font(GaryxFont.fixedSystem(size: iconSize, weight: iconWeight))
            .foregroundStyle(foreground)
            .frame(width: diameter, height: diameter)
            .background(background, in: Circle())
    }
}

struct GaryxPrimaryCapsuleButton: View {
    @ScaledMetric(relativeTo: .body) private var verticalPadding: CGFloat = 16
    let title: String
    var systemImage: String? = nil
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                if let systemImage, !systemImage.isEmpty {
                    Image(systemName: systemImage)
                        .font(GaryxFont.fixedSystem(size: 15, weight: .semibold))
                }

                Text(title)
                    .font(GaryxFont.body(weight: .semibold))
            }
            .foregroundStyle(Color(.systemBackground))
            .frame(maxWidth: .infinity)
            .padding(.vertical, verticalPadding)
            .frame(minHeight: 56)
            .background(Color(.label), in: Capsule())
        }
        .buttonStyle(GaryxPressableRowStyle())
    }
}
