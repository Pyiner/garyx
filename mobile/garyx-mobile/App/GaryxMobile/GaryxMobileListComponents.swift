import Foundation
import SwiftUI
import UIKit

struct GaryxSectionBlock<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            GaryxFieldLabel(title)
            VStack(alignment: .leading, spacing: 10) {
                content
            }
        }
    }
}

struct GaryxCompactListGroup<Content: View>: View {
    let content: Content

    init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(GaryxTheme.surface)
    }
}

struct GaryxCompactRowDivider: View {
    var body: some View {
        Divider()
            .overlay(GaryxTheme.hairline)
            .padding(.leading, 10)
    }
}

struct GaryxCompactGroupDivider: View {
    var body: some View {
        VStack(spacing: 0) {
            Divider()
                .overlay(GaryxTheme.hairline)
            GaryxTheme.background
                .frame(height: 7)
            Divider()
                .overlay(GaryxTheme.hairline)
        }
    }
}

struct GaryxDisclosureListRow: View {
    let title: String
    var subtitle: String?
    var systemImage: String?
    var selectedSystemImage: String?
    var isSelected = false
    var iconFrame: CGFloat = 28
    var horizontalPadding: CGFloat = 16
    var verticalPadding: CGFloat = 9
    var minHeight: CGFloat = 52
    var titleWeight: Font.Weight = .semibold
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                if let imageName {
                    Image(systemName: imageName)
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(isSelected ? .primary : .secondary)
                        .frame(width: iconFrame, height: iconFrame)
                }

                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .font(GaryxFont.subheadline(weight: titleWeight))
                        .foregroundStyle(.primary)
                        .lineLimit(1)

                    if let subtitle, !subtitle.isEmpty {
                        Text(subtitle)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }

                Spacer(minLength: 0)

                Image(systemName: "chevron.right")
                    .font(GaryxFont.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, horizontalPadding)
            .padding(.vertical, verticalPadding)
            .frame(minHeight: minHeight)
            .background {
                if isSelected {
                    Color(.tertiarySystemFill).opacity(0.56)
                        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
    }

    private var imageName: String? {
        isSelected ? (selectedSystemImage ?? systemImage) : systemImage
    }
}

struct GaryxSelectionCheckmark: View {
    enum Style {
        case plain
        case circle
    }

    var style: Style = .plain
    var size: CGFloat = 14
    var weight: Font.Weight = .semibold

    var body: some View {
        Image(systemName: systemName)
            .font(GaryxFont.system(size: size, weight: weight))
            .foregroundStyle(.primary)
            .accessibilityHidden(true)
    }

    private var systemName: String {
        switch style {
        case .plain:
            "checkmark"
        case .circle:
            "checkmark.circle.fill"
        }
    }
}

struct GaryxMenuSelectionLabel: View {
    let title: String
    let selected: Bool
    let fallbackSystemImage: String

    var body: some View {
        Label {
            Text(title)
        } icon: {
            if selected {
                GaryxSelectionCheckmark(size: 13)
            } else {
                Image(systemName: fallbackSystemImage)
                    .font(GaryxFont.system(size: 13, weight: .semibold))
                    .foregroundStyle(.secondary)
            }
        }
    }
}

/// Row-level secondary actions rendered as a trailing ellipsis menu.
/// Horizontal row swipes are reserved for navigation/sidebar gestures.
struct GaryxRowAction {
    enum Tone {
        case accent
        case neutral
        case warning
        case destructive

        var background: Color {
            switch self {
            case .accent:
                GaryxTheme.accent
            case .neutral:
                Color(.systemGray3)
            case .warning:
                GaryxTheme.warning
            case .destructive:
                GaryxTheme.danger
            }
        }
    }

    let title: String
    let systemImage: String
    var tone: Tone = .neutral
    let action: () -> Void
}

struct GaryxRowActionMenu<Content: View>: View {
    let actions: [GaryxRowAction]
    let content: Content
    private let actionMenuWidth: CGFloat = 36
    private let actionMenuTrailingInset: CGFloat = 10
    private let actionMenuContentGap: CGFloat = 8

    init(actions: [GaryxRowAction], @ViewBuilder content: () -> Content) {
        self.actions = actions
        self.content = content()
    }

    var body: some View {
        if actions.isEmpty {
            content
        } else {
            content
                .padding(.trailing, actionMenuWidth + actionMenuTrailingInset + actionMenuContentGap)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(GaryxTheme.surface)
                .contentShape(Rectangle())
                .accessibilityHint("Use the actions button for item actions.")
                .modifier(GaryxRowMenuAccessibilityActions(actions: actions, onAction: handle))
                .overlay(alignment: .trailing) {
                    Menu {
                        ForEach(Array(actions.enumerated()), id: \.offset) { _, action in
                            Button(role: action.menuRole) {
                                handle(action)
                            } label: {
                                Label(action.title, systemImage: action.systemImage)
                            }
                        }
                    } label: {
                        Image(systemName: "ellipsis")
                            .font(GaryxFont.system(size: 17, weight: .semibold))
                            .foregroundStyle(.secondary)
                            .frame(width: actionMenuWidth, height: 28)
                            .garyxAdaptiveGlass(
                                .regular,
                                isInteractive: true,
                                tint: Color(.systemBackground).opacity(0.68),
                                fallbackMaterial: .ultraThinMaterial,
                                in: Capsule()
                            )
                            .contentShape(Capsule())
                    }
                    .buttonStyle(GaryxItemActionMenuButtonStyle())
                    .padding(.trailing, actionMenuTrailingInset)
                    .accessibilityLabel("Item actions")
                }
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
        }
    }

    private func handle(_ action: GaryxRowAction) {
        action.action()
    }
}

private struct GaryxOpenSwipeActionRowIdKey: EnvironmentKey {
    static let defaultValue: Binding<String?> = .constant(nil)
}

extension EnvironmentValues {
    var garyxOpenSwipeActionRowId: Binding<String?> {
        get { self[GaryxOpenSwipeActionRowIdKey.self] }
        set { self[GaryxOpenSwipeActionRowIdKey.self] = newValue }
    }
}

struct GaryxSwipeActionRow<Content: View>: View {
    var id: String?
    let actions: [GaryxRowAction]
    let content: Content
    @Environment(\.garyxOpenSwipeActionRowId) private var openSwipeActionRowId
    @GestureState private var dragTranslation: CGFloat = 0
    @State private var localIsOpen = false
    @State private var didPlayFullRevealFeedback = false

    private let actionButtonDiameter: CGFloat = 38
    private let actionButtonSpacing: CGFloat = 10
    private let actionTrailingPadding: CGFloat = 10

    init(id: String? = nil, actions: [GaryxRowAction], @ViewBuilder content: () -> Content) {
        self.id = id
        self.actions = actions
        self.content = content()
    }

    var body: some View {
        if actions.isEmpty {
            content
        } else {
            ZStack(alignment: .trailing) {
                actionButtons

                content
                    .background(GaryxTheme.surface)
                    .offset(x: currentOffset)
                    .animation(GaryxMobileMotion.rowSwipe, value: isOpen)
            }
            .contentShape(Rectangle())
            .clipped()
            .simultaneousGesture(rowDragGesture)
            .onChange(of: isOpen) { _, open in
                if !open {
                    didPlayFullRevealFeedback = false
                }
            }
            .accessibilityHint("Swipe left for thread actions.")
            .modifier(GaryxRowMenuAccessibilityActions(actions: actions, onAction: perform))
        }
    }

    private var actionButtons: some View {
        HStack(spacing: actionButtonSpacing) {
            ForEach(Array(actions.enumerated()), id: \.offset) { _, action in
                Button(role: action.menuRole) {
                    perform(action)
                } label: {
                    Image(systemName: action.systemImage)
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                        .foregroundStyle(Color.white)
                        .rotationEffect(.degrees(action.iconRotationDegrees))
                        .frame(width: actionButtonDiameter, height: actionButtonDiameter)
                        .background(action.tone.background, in: Circle())
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(action.title)
            }
        }
        .padding(.trailing, actionTrailingPadding)
        .frame(width: maxRevealWidth)
    }

    private var rowDragGesture: some Gesture {
        DragGesture(minimumDistance: 12, coordinateSpace: .local)
            .updating($dragTranslation) { value, state, _ in
                state = horizontalSwipeTranslation(value.translation)
            }
            .onChanged { value in
                let translation = horizontalSwipeTranslation(value.translation)
                closeOtherOpenRowIfNeeded(translation: translation)
                updateFullRevealFeedback(for: clampedOffset(baseOffset + translation))
            }
            .onEnded { value in
                let translation = horizontalSwipeTranslation(value.translation)
                guard translation != 0 || isOpen else { return }
                let predicted = horizontalSwipeTranslation(value.predictedEndTranslation)
                let projectedOffset = clampedOffset(baseOffset + (predicted == 0 ? translation : predicted))
                let nextIsOpen = projectedOffset < -maxRevealWidth * 0.35
                withAnimation(GaryxMobileMotion.rowSwipe) {
                    setOpen(nextIsOpen)
                }
                if nextIsOpen {
                    playFullRevealFeedbackIfNeeded()
                } else {
                    didPlayFullRevealFeedback = false
                }
            }
    }

    private var currentOffset: CGFloat {
        clampedOffset(baseOffset + dragTranslation)
    }

    private var baseOffset: CGFloat {
        isOpen ? -maxRevealWidth : 0
    }

    private var isOpen: Bool {
        if let id {
            return openSwipeActionRowId.wrappedValue == id
        }
        return localIsOpen
    }

    private var maxRevealWidth: CGFloat {
        CGFloat(actions.count) * actionButtonDiameter
            + CGFloat(max(0, actions.count - 1)) * actionButtonSpacing
            + actionTrailingPadding
    }

    private func horizontalSwipeTranslation(_ translation: CGSize) -> CGFloat {
        let horizontal = translation.width
        let vertical = translation.height
        let horizontalMagnitude = abs(horizontal)
        let verticalMagnitude = abs(vertical)
        guard horizontalMagnitude > verticalMagnitude * 1.15 else { return 0 }
        if !isOpen {
            return min(0, horizontal)
        }
        return horizontal
    }

    private func clampedOffset(_ value: CGFloat) -> CGFloat {
        min(0, max(-maxRevealWidth, value))
    }

    private func perform(_ action: GaryxRowAction) {
        withAnimation(GaryxMobileMotion.rowSwipe) {
            setOpen(false)
        }
        action.action()
    }

    private func setOpen(_ open: Bool) {
        if let id {
            if open {
                openSwipeActionRowId.wrappedValue = id
            } else if openSwipeActionRowId.wrappedValue == id {
                openSwipeActionRowId.wrappedValue = nil
            }
        } else {
            localIsOpen = open
        }
    }

    private func closeOtherOpenRowIfNeeded(translation: CGFloat) {
        guard let id,
              translation < -4,
              let openId = openSwipeActionRowId.wrappedValue,
              openId != id else {
            return
        }
        withAnimation(GaryxMobileMotion.rowSwipe) {
            openSwipeActionRowId.wrappedValue = nil
        }
    }

    private func updateFullRevealFeedback(for offset: CGFloat) {
        if offset <= -maxRevealWidth + 0.5 {
            playFullRevealFeedbackIfNeeded()
        } else if offset > -maxRevealWidth + 8 {
            didPlayFullRevealFeedback = false
        }
    }

    private func playFullRevealFeedbackIfNeeded() {
        guard !didPlayFullRevealFeedback else { return }
        didPlayFullRevealFeedback = true
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
    }
}

struct GaryxItemActionMenuButtonStyle: ButtonStyle {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed && !reduceMotion ? 0.96 : 1)
            .opacity(configuration.isPressed ? 0.78 : 1)
            .animation(reduceMotion ? nil : .easeOut(duration: 0.12), value: configuration.isPressed)
    }
}

private extension GaryxRowAction {
    var menuRole: ButtonRole? {
        tone == .destructive ? .destructive : nil
    }

    var iconRotationDegrees: Double {
        systemImage.hasPrefix("pin") ? -28 : 0
    }
}

private struct GaryxRowMenuAccessibilityActions: ViewModifier {
    let actions: [GaryxRowAction]
    let onAction: (GaryxRowAction) -> Void

    func body(content: Content) -> some View {
        content.accessibilityActions {
            ForEach(Array(actions.enumerated()), id: \.offset) { _, action in
                Button(action.title) {
                    onAction(action)
                }
            }
        }
    }
}

struct GaryxCompactInfoRow: View {
    let title: String
    let subtitle: String
    let iconName: String

    var body: some View {
        HStack(spacing: 9) {
            Image(systemName: iconName)
                .font(GaryxFont.system(size: 14, weight: .medium))
                .foregroundStyle(.secondary)
                .frame(width: 20, height: 20)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                if !subtitle.isEmpty {
                    Text(subtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 7)
    }
}
