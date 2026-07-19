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
    @Environment(\.layoutDirection) private var layoutDirection
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.garyxPrefersCrossFadeTransitions) private var prefersCrossFadeTransitions
    @StateObject private var revealInteraction = GaryxHorizontalRevealInteractionStore(
        projection: .shortTravelDismiss
    )
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
                if revealInteraction.reveal > 0 {
                    actionButtons
                        .allowsHitTesting(isOpen)
                        .zIndex(isOpen ? 1 : 0)
                }

                content
                    .background(GaryxTheme.surface)
                    .offset(x: physicalContentOffset)
            }
            .contentShape(Rectangle())
            .clipped()
            .gesture(rowPanGesture)
            .onAppear {
                revealInteraction.configure(
                    extent: maxRevealWidth,
                    restingPosition: isOpen ? .open : .closed
                )
            }
            .onChange(of: maxRevealWidth) { oldWidth, newWidth in
                guard oldWidth != newWidth else { return }
                revealInteraction.configure(
                    extent: newWidth,
                    restingPosition: isOpen ? .open : .closed
                )
            }
            .onChange(of: isOpen) { _, open in
                revealInteraction.setTarget(
                    open ? .open : .closed,
                    animated: animatesTransitions
                )
                if !open {
                    didPlayFullRevealFeedback = false
                }
            }
            .accessibilityHint(
                layoutDirection == .leftToRight
                    ? "Swipe left for thread actions."
                    : "Swipe right for thread actions."
            )
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

    private var rowPanGesture: GaryxHorizontalPanGesture {
        GaryxHorizontalPanGesture(
            shouldBegin: { translation, velocity in
                let intent = velocity == .zero ? translation.width : velocity.width
                let logicalIntent = GaryxRouteEdgeGestureArbitrator.logicalTranslation(
                    physicalTranslationX: intent,
                    edge: .trailing,
                    layoutDirection: routeLayoutDirection
                )
                return revealInteraction.acceptedDirection.accepts(logicalIntent)
            },
            onBegan: {
                revealInteraction.beginGesture()
            },
            onChanged: { translation, _ in
                let logicalTranslation = logicalRevealTranslation(translation.width)
                closeOtherOpenRowIfNeeded(translation: logicalTranslation)
                revealInteraction.updateGesture(logicalTranslation: logicalTranslation)
                updateFullRevealFeedback(for: revealInteraction.reveal)
            },
            onEnded: { translation, velocity in
                revealInteraction.updateGesture(
                    logicalTranslation: logicalRevealTranslation(translation.width)
                )
                guard let target = revealInteraction.endGesture(
                    logicalVelocity: logicalRevealTranslation(velocity.width)
                ) else { return }
                let nextIsOpen = target == .open
                setOpen(nextIsOpen)
                if nextIsOpen {
                    playFullRevealFeedbackIfNeeded()
                } else {
                    didPlayFullRevealFeedback = false
                }
            },
            onCancelled: {
                guard let target = revealInteraction.cancelGesture() else { return }
                setOpen(target == .open)
            }
        )
    }

    private var physicalContentOffset: CGFloat {
        let leadingSign: CGFloat = layoutDirection == .leftToRight ? 1 : -1
        return -leadingSign * revealInteraction.reveal
    }

    private var routeLayoutDirection: GaryxRouteLayoutDirection {
        layoutDirection == .leftToRight ? .leftToRight : .rightToLeft
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

    private func logicalRevealTranslation(_ physicalTranslation: CGFloat) -> CGFloat {
        GaryxRouteEdgeGestureArbitrator.logicalTranslation(
            physicalTranslationX: physicalTranslation,
            edge: .trailing,
            layoutDirection: routeLayoutDirection
        )
    }

    private func perform(_ action: GaryxRowAction) {
        revealInteraction.setTarget(.closed, animated: animatesTransitions)
        setOpen(false)
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
              translation > 4,
              let openId = openSwipeActionRowId.wrappedValue,
              openId != id else {
            return
        }
        openSwipeActionRowId.wrappedValue = nil
    }

    private func updateFullRevealFeedback(for reveal: CGFloat) {
        if reveal >= maxRevealWidth - 0.5 {
            playFullRevealFeedbackIfNeeded()
        } else if reveal < maxRevealWidth - 8 {
            didPlayFullRevealFeedback = false
        }
    }

    private var animatesTransitions: Bool {
        GaryxAccessibilityTransitionPolicy.animatesTransition(
            reduceMotion: reduceMotion,
            prefersCrossFadeTransitions: prefersCrossFadeTransitions
        )
    }

    private func playFullRevealFeedbackIfNeeded() {
        guard !didPlayFullRevealFeedback else { return }
        didPlayFullRevealFeedback = true
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
    }
}

struct GaryxItemActionMenuButtonStyle: ButtonStyle {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.garyxPrefersCrossFadeTransitions) private var prefersCrossFadeTransitions

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed && !usesCrossFade ? 0.96 : 1)
            .opacity(configuration.isPressed ? 0.78 : 1)
            .animation(pressAnimation, value: configuration.isPressed)
    }

    private var usesCrossFade: Bool {
        GaryxAccessibilityTransitionPolicy.usesCrossFade(
            reduceMotion: reduceMotion,
            prefersCrossFadeTransitions: prefersCrossFadeTransitions
        )
    }

    private var pressAnimation: Animation? {
        GaryxAccessibilityTransitionPolicy.animatesTransition(
            reduceMotion: reduceMotion,
            prefersCrossFadeTransitions: prefersCrossFadeTransitions
        ) ? .easeOut(duration: 0.12) : nil
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
