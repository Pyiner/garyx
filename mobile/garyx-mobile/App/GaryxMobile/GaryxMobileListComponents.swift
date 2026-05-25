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
