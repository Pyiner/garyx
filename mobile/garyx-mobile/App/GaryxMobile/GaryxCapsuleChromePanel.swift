import SwiftUI

struct GaryxCapsuleChromeAnchorKey: PreferenceKey {
    static var defaultValue: Anchor<CGRect>?

    static func reduce(value: inout Anchor<CGRect>?, nextValue: () -> Anchor<CGRect>?) {
        value = nextValue() ?? value
    }
}

enum GaryxCapsuleChromeMetrics {
    static let metrics = GaryxChromeMorphSurfaceMetrics(
        horizontalMargin: 12,
        maximumExpandedWidth: 480,
        collapsedCornerRadius: 22,
        expandedCornerRadius: 28
    )
}

struct GaryxCapsuleChromeCompactRow: View {
    let title: String
    var maxWidth: CGFloat? = 282

    var body: some View {
        HStack(spacing: 8) {
            GaryxCapsuleGlyph()
                .frame(width: 22, height: 22)
                .foregroundStyle(.secondary)
            Text(title)
                .font(GaryxFont.callout(weight: .medium))
                .foregroundStyle(.primary)
                .lineLimit(1)
                .truncationMode(.tail)
                .layoutPriority(1)
        }
        .padding(.horizontal, 12)
        .frame(height: 44, alignment: .leading)
        .frame(maxWidth: maxWidth ?? .infinity, alignment: .leading)
    }
}

/// Publishes a Capsule-only anchor and preserves its layout while the morph
/// twin is mounted, matching the thread title's no-reflow behavior.
struct GaryxCapsuleChromeHeaderControl: View {
    let title: String
    let isHidden: Bool
    let onToggle: () -> Void

    var body: some View {
        Button(action: onToggle) {
            // Liquid Glass is direct on the interactive label; the explicit
            // content shape makes the full capsule tappable on iOS 26.
            GaryxCapsuleChromeCompactRow(title: title)
                .garyxAdaptiveGlass(
                    .regular,
                    isInteractive: false,
                    in: Capsule(),
                    isEnabled: !isHidden
                )
                .contentShape(Capsule())
        }
        .buttonStyle(GaryxPressableRowStyle())
        .opacity(isHidden ? 0 : 1)
        .allowsHitTesting(!isHidden)
        .accessibilityLabel("\(title), Capsule actions")
        .accessibilityHidden(isHidden)
        .anchorPreference(key: GaryxCapsuleChromeAnchorKey.self, value: .bounds) { $0 }
        .layoutPriority(1)
    }
}

enum GaryxCapsuleChromeAction: Equatable {
    case openSourceConversation
    case copyLink
    case copyID
    case delete
}

struct GaryxCapsuleChromePanel: View {
    @EnvironmentObject private var model: GaryxMobileModel

    let capsule: GaryxCapsuleSummary
    let sourceThread: GaryxThreadSummary?
    let compactRowWidth: CGFloat
    let isExpanded: Bool
    let onToggle: () -> Void
    let onAction: (GaryxCapsuleChromeAction) -> Void

    private var sourceThreadId: String? {
        guard let id = capsule.threadId?.trimmingCharacters(in: .whitespacesAndNewlines),
              !id.isEmpty else { return nil }
        return id
    }

    private var sourceConversationTitle: String {
        let title = sourceThread?.title.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return title.isEmpty ? "Source conversation" : title
    }

    private var sourceConversationAvatar: GaryxSidebarThreadRowAvatar {
        if let sourceThread {
            let identity = model.widgetAgentIdentity(for: sourceThread)
            return GaryxSidebarThreadRowAvatar(
                agentId: identity.id ?? "",
                avatarDataUrl: identity.avatarDataUrl ?? "",
                label: identity.name ?? sourceConversationTitle,
                providerType: identity.providerType ?? sourceThread.providerType ?? "",
                builtIn: identity.builtIn
            )
        }

        let capsuleAgentId = capsule.agentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let target = model.agentTargets.first { $0.id == capsuleAgentId }
        return GaryxSidebarThreadRowAvatar(
            agentId: target?.id ?? capsuleAgentId,
            avatarDataUrl: target?.avatarDataUrl ?? "",
            label: target?.title ?? sourceConversationTitle,
            providerType: target?.providerType ?? capsule.providerType ?? "",
            builtIn: target?.builtIn ?? false
        )
    }

    var body: some View {
        VStack(spacing: 0) {
            Button(action: onToggle) {
                GaryxCapsuleChromeCompactRow(title: capsule.displayTitle, maxWidth: nil)
                    .frame(width: isExpanded ? nil : compactRowWidth, alignment: .leading)
                    .contentShape(Rectangle())
                    .clipped()
            }
            .buttonStyle(GaryxPressableRowStyle())
            .accessibilityLabel("Close Capsule actions")

            if isExpanded {
                Divider().opacity(0.65)
                VStack(spacing: 0) {
                    if sourceThreadId != nil {
                        sourceConversationRow
                        Divider().padding(.leading, 60).opacity(0.55)
                    }
                    actionRow("Copy Link", systemName: "link", action: .copyLink)
                    Divider().padding(.leading, 52).opacity(0.55)
                    actionRow("Copy ID", systemName: "number", action: .copyID)
                    Divider().padding(.leading, 52).opacity(0.55)
                    actionRow("Delete", systemName: "trash", action: .delete, destructive: true)
                }
                .padding(.horizontal, 12)
                .padding(.bottom, 12)
                .transition(.opacity)
            }
        }
        .opacity(isExpanded ? 1 : 0.999)
    }

    private var sourceConversationRow: some View {
        Button { onAction(.openSourceConversation) } label: {
            HStack(spacing: 10) {
                GaryxAgentAvatarView(
                    agentId: sourceConversationAvatar.agentId,
                    avatarDataUrl: sourceConversationAvatar.avatarDataUrl,
                    label: sourceConversationAvatar.label,
                    providerType: sourceConversationAvatar.providerType,
                    builtIn: sourceConversationAvatar.builtIn,
                    diameter: 38
                )

                Text(sourceConversationTitle)
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .layoutPriority(1)

                Spacer(minLength: 0)
            }
            .frame(minHeight: 62)
            .contentShape(Rectangle())
        }
        .buttonStyle(GaryxPressableRowStyle())
        .accessibilityLabel("Open source conversation, \(sourceConversationTitle)")
    }

    private func actionRow(
        _ title: String,
        systemName: String,
        action: GaryxCapsuleChromeAction,
        destructive: Bool = false
    ) -> some View {
        Button { onAction(action) } label: {
            HStack(spacing: 12) {
                Image(systemName: systemName)
                    .font(GaryxFont.system(size: 16, weight: .medium))
                    .frame(width: 24)
                Text(title)
                    .font(GaryxFont.scaledCallout(weight: .medium))
                Spacer(minLength: 0)
            }
            .foregroundStyle(destructive ? GaryxTheme.danger : Color.primary)
            .frame(minHeight: 50)
            .contentShape(Rectangle())
        }
        .buttonStyle(GaryxPressableRowStyle())
    }
}
