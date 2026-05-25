import Foundation
import SwiftUI
import UIKit

private struct GaryxOpenSidebarActionKey: EnvironmentKey {
    static let defaultValue: () -> Void = {}
}

extension EnvironmentValues {
    var garyxOpenSidebar: () -> Void {
        get { self[GaryxOpenSidebarActionKey.self] }
        set { self[GaryxOpenSidebarActionKey.self] = newValue }
    }
}

enum GaryxDataURLImageCache {
    private static let cache: NSCache<NSString, UIImage> = {
        let cache = NSCache<NSString, UIImage>()
        cache.countLimit = 128
        cache.totalCostLimit = 32 * 1024 * 1024
        return cache
    }()

    static func image(from rawValue: String?) -> UIImage? {
        let raw = (rawValue ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty else { return nil }
        let cacheKey = NSString(string: raw)
        if let cached = cache.object(forKey: cacheKey) {
            return cached
        }
        let encoded = raw.split(separator: ",", maxSplits: 1).last.map(String.init) ?? raw
        guard let data = Data(base64Encoded: encoded),
              let image = UIImage(data: data) else {
            return nil
        }
        cache.setObject(image, forKey: cacheKey, cost: data.count)
        return image
    }
}

struct GaryxPanelScaffold<Content: View, Actions: View>: View {
    @Environment(\.garyxOpenSidebar) private var openSidebar

    let title: String
    let subtitle: String
    let onRefresh: (() async -> Void)?
    let showsRefreshButton: Bool
    let leadingActionLabel: String?
    let leadingActionSystemName: String
    let leadingAction: (() -> Void)?
    let background: Color
    let content: Content
    let actions: Actions

    init(
        title: String,
        subtitle: String,
        onRefresh: (() async -> Void)? = nil,
        showsRefreshButton: Bool? = nil,
        leadingActionLabel: String? = nil,
        leadingActionSystemName: String = "chevron.left",
        leadingAction: (() -> Void)? = nil,
        background: Color = GaryxTheme.background,
        @ViewBuilder content: () -> Content,
        @ViewBuilder actions: () -> Actions
    ) {
        self.title = title
        self.subtitle = subtitle
        self.onRefresh = onRefresh
        self.showsRefreshButton = showsRefreshButton ?? (onRefresh != nil)
        self.leadingActionLabel = leadingActionLabel
        self.leadingActionSystemName = leadingActionSystemName
        self.leadingAction = leadingAction
        self.background = background
        self.content = content()
        self.actions = actions()
    }

    var body: some View {
        ScrollView {
            content
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
                .frame(maxWidth: 560, alignment: .leading)
                .frame(maxWidth: .infinity)
        }
        .refreshable {
            if let onRefresh {
                await onRefresh()
            }
        }
        .background(background)
        .garyxAdaptiveTopBar {
            GaryxAdaptiveGlassContainer(spacing: 10) {
                HStack(spacing: 12) {
                    if let leadingAction {
                        Button {
                            leadingAction()
                        } label: {
                            GaryxToolbarIcon(systemName: leadingActionSystemName)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(leadingActionLabel ?? "Back")
                    } else {
                        GaryxSidebarMenuButton {
                            openSidebar()
                        }
                    }

                    GaryxPanelHeaderTitle(title: title, subtitle: subtitle)
                        .layoutPriority(1)

                    Spacer(minLength: 0)

                    if let onRefresh, showsRefreshButton {
                        Button {
                            Task { await onRefresh() }
                        } label: {
                            GaryxToolbarIcon(systemName: "arrow.clockwise")
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Refresh")
                    }

                    actions
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 10)
            .padding(.bottom, 8)
        }
    }
}

struct GaryxPanelHeaderTitle: View {
    let title: String

    init(title: String, subtitle: String = "") {
        self.title = title
    }

    var body: some View {
        Text(title)
            .font(GaryxFont.callout(weight: .medium))
            .foregroundStyle(.primary)
            .lineLimit(1)
            .truncationMode(.tail)
            .padding(.horizontal, 14)
            .frame(height: 44, alignment: .leading)
            .garyxAdaptiveGlass(
                .regular,
                isInteractive: false,
                fallbackMaterial: .ultraThinMaterial,
                in: Capsule()
            )
    }
}

extension GaryxPanelScaffold where Actions == EmptyView {
    init(
        title: String,
        subtitle: String,
        onRefresh: (() async -> Void)? = nil,
        showsRefreshButton: Bool? = nil,
        leadingActionLabel: String? = nil,
        leadingActionSystemName: String = "chevron.left",
        leadingAction: (() -> Void)? = nil,
        background: Color = GaryxTheme.background,
        @ViewBuilder content: () -> Content
    ) {
        self.init(
            title: title,
            subtitle: subtitle,
            onRefresh: onRefresh,
            showsRefreshButton: showsRefreshButton,
            leadingActionLabel: leadingActionLabel,
            leadingActionSystemName: leadingActionSystemName,
            leadingAction: leadingAction,
            background: background,
            content: content,
            actions: { EmptyView() }
        )
    }
}

struct GaryxAddToolbarButton: View {
    let label: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            GaryxToolbarIcon(systemName: "plus")
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}

struct GaryxFormSheet<Content: View>: View {
    @Environment(\.dismiss) private var dismiss
    let title: String
    let canSave: Bool?
    let onCancel: (() -> Void)?
    let onSave: (() -> Void)?
    let onDone: (() -> Void)?
    let content: Content

    init(title: String, onDone: (() -> Void)? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.canSave = nil
        self.onCancel = nil
        self.onSave = nil
        self.onDone = onDone
        self.content = content()
    }

    init(
        title: String,
        canSave: Bool,
        onCancel: (() -> Void)? = nil,
        onSave: @escaping () -> Void,
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.canSave = canSave
        self.onCancel = onCancel
        self.onSave = onSave
        self.onDone = nil
        self.content = content()
    }

    var body: some View {
        ZStack(alignment: .top) {
            GaryxFormPalette.pageBackground
                .ignoresSafeArea()

            ScrollView {
                content
                    .padding(.horizontal, 18)
                    .padding(.top, 92)
                    .padding(.bottom, 28)
                    .frame(maxWidth: 560, alignment: .leading)
                    .frame(maxWidth: .infinity)
            }

            ZStack {
                HStack {
                    Button(action: cancel) {
                        GaryxToolbarIcon(systemName: "xmark")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Cancel")

                    Spacer(minLength: 0)

                    if let onSave {
                        Button(action: onSave) {
                            GaryxToolbarIcon(systemName: "checkmark")
                                .opacity(canSave == false ? 0.42 : 1)
                        }
                        .buttonStyle(.plain)
                        .disabled(canSave == false)
                        .accessibilityLabel("Save")
                    }
                }

                Text(title)
                    .font(GaryxFont.title3(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
            }
            .padding(.horizontal, 18)
            .padding(.top, 10)
        }
    }

    private func cancel() {
        if let onCancel {
            onCancel()
        } else if let onDone {
            onDone()
        } else {
            dismiss()
        }
    }
}

struct GaryxFormGroupedSection<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(.secondary)
                .textCase(.uppercase)
                .padding(.horizontal, 14)

            VStack(alignment: .leading, spacing: 0) {
                content
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(GaryxFormPalette.cardBackground, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        }
    }
}

struct GaryxFormRow<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        HStack(spacing: 12) {
            Text(title)
                .font(GaryxFont.body())
                .foregroundStyle(.primary)
            Spacer(minLength: 8)
            content
                .font(GaryxFont.body())
                .foregroundStyle(.primary)
                .multilineTextAlignment(.trailing)
        }
        .padding(.horizontal, 16)
        .frame(minHeight: 52)
    }
}

struct GaryxFormReadOnlyRow: View {
    let title: String
    let value: String

    var body: some View {
        GaryxFormRow(title: title) {
            Text(value)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
        }
    }
}

struct GaryxFormMenuValueLabel: View {
    let value: String

    var body: some View {
        HStack(spacing: 6) {
            Text(value)
                .font(GaryxFont.body(weight: .medium))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            Image(systemName: "chevron.up.chevron.down")
                .font(GaryxFont.system(size: 11, weight: .semibold))
                .foregroundStyle(.tertiary)
        }
        .fixedSize(horizontal: false, vertical: true)
    }
}

struct GaryxFormSelectionRow: View {
    let title: String
    let value: String
    let placeholder: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Text(title)
                    .font(GaryxFont.body())
                    .foregroundStyle(.primary)
                Spacer(minLength: 8)
                Text(displayValue)
                    .font(GaryxFont.body())
                    .foregroundStyle(isPlaceholder ? .secondary : .primary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Image(systemName: "chevron.up.chevron.down")
                    .font(GaryxFont.system(size: 14, weight: .semibold))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 16)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private var displayValue: String {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? placeholder : value
    }

    private var isPlaceholder: Bool {
        value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

struct GaryxFormErrorText: View {
    let text: String

    var body: some View {
        Text(text)
            .font(GaryxFont.caption(weight: .medium))
            .foregroundStyle(GaryxTheme.danger)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, 14)
    }
}

enum GaryxFormPalette {
    static let pageBackground = Color(.systemGroupedBackground).opacity(0.72)
    static let cardBackground = Color(.systemBackground)
}

extension View {
    func garyxFormTextField(minHeight: CGFloat = 52, horizontalPadding: CGFloat = 16) -> some View {
        self
            .font(GaryxFont.body())
            .foregroundStyle(.primary)
            .padding(.horizontal, horizontalPadding)
            .frame(minHeight: minHeight, alignment: .leading)
    }

    func garyxFormTextArea(minHeight: CGFloat = 132) -> some View {
        self
            .font(GaryxFont.body())
            .foregroundStyle(.primary)
            .padding(16)
            .frame(minHeight: minHeight, alignment: .topLeading)
    }
}

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

struct GaryxSwipeAction {
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

struct GaryxSwipeActionRow<Content: View>: View {
    let actions: [GaryxSwipeAction]
    let content: Content
    private let actionMenuWidth: CGFloat = 36
    private let actionMenuTrailingInset: CGFloat = 10
    private let actionMenuContentGap: CGFloat = 8

    init(actions: [GaryxSwipeAction], @ViewBuilder content: () -> Content) {
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
                .modifier(GaryxSwipeRowAccessibilityActions(actions: actions, onAction: handle))
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

    private func handle(_ action: GaryxSwipeAction) {
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

private extension GaryxSwipeAction {
    var menuRole: ButtonRole? {
        tone == .destructive ? .destructive : nil
    }
}

private struct GaryxSwipeRowAccessibilityActions: ViewModifier {
    let actions: [GaryxSwipeAction]
    let onAction: (GaryxSwipeAction) -> Void

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

struct GaryxChannelLogoView: View {
    let channel: String
    let label: String
    let iconDataUrl: String?
    var diameter: CGFloat = 30

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: diameter * 0.28, style: .continuous)
                .fill(Color(.secondarySystemFill))

            if let image = decodedImage {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFit()
                    .padding(diameter * 0.16)
            } else if let image = builtInFallbackImage {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFit()
                    .padding(diameter * 0.16)
            } else {
                Text(fallbackLabel)
                    .font(GaryxFont.system(size: diameter * 0.34, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.65)
            }
        }
        .frame(width: diameter, height: diameter)
        .overlay {
            RoundedRectangle(cornerRadius: diameter * 0.28, style: .continuous)
                .stroke(Color.primary.opacity(0.06), lineWidth: 1)
        }
        .accessibilityHidden(true)
    }

    private var decodedImage: UIImage? {
        GaryxDataURLImageCache.image(from: iconDataUrl)
    }

    private var builtInFallbackImage: UIImage? {
        switch channel.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "telegram":
            UIImage(named: "ChannelTelegram")
        case "discord":
            UIImage(named: "ChannelDiscord")
        case "feishu":
            UIImage(named: "ChannelFeishu")
        case "weixin":
            UIImage(named: "ChannelWeixin")
        default:
            nil
        }
    }

    private var fallbackLabel: String {
        let source = label.isEmpty ? channel : label
        let words = source
            .replacingOccurrences(of: "_", with: " ")
            .replacingOccurrences(of: "-", with: " ")
            .split(separator: " ")
        let initials = words.prefix(2).compactMap { $0.first }.map { String($0).uppercased() }.joined()
        return initials.isEmpty ? "B" : initials
    }
}

private enum GaryxProviderAvatar {
    case codex
    case openAI
    case claude
    case gemini
    case generic

    var symbol: String? {
        switch self {
        case .codex:
            "chevron.left.forwardslash.chevron.right"
        case .openAI:
            "circle.hexagongrid.fill"
        case .claude:
            "sparkles"
        case .gemini:
            "diamond.fill"
        case .generic:
            nil
        }
    }

    var background: Color {
        switch self {
        case .codex:
            Color(red: 0.08, green: 0.10, blue: 0.12)
        case .openAI:
            Color(red: 0.10, green: 0.47, blue: 0.40)
        case .claude:
            Color(red: 0.50, green: 0.37, blue: 0.26)
        case .gemini:
            Color(red: 0.23, green: 0.38, blue: 0.86)
        case .generic:
            Color(.secondarySystemBackground)
        }
    }

    var foreground: Color {
        switch self {
        case .generic:
            Color(.secondaryLabel)
        default:
            Color.white
        }
    }

    func iconSize(for diameter: CGFloat) -> CGFloat {
        switch self {
        case .codex:
            diameter * 0.32
        case .openAI:
            diameter * 0.42
        case .claude:
            diameter * 0.40
        case .gemini:
            diameter * 0.34
        case .generic:
            diameter * 0.36
        }
    }
}

struct GaryxAgentAvatarView: View {
    let agentId: String
    let avatarDataUrl: String
    let kind: GaryxMobileAgentTarget.Kind
    let label: String
    let providerType: String
    var builtIn: Bool = false
    var diameter: CGFloat = 34

    var body: some View {
        ZStack {
            Circle()
                .fill(fallbackBackground)
            if let image = decodedImage {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
                    .frame(width: diameter, height: diameter)
                    .clipShape(Circle())
            } else if let remoteAvatarURL {
                AsyncImage(url: remoteAvatarURL) { phase in
                    if let image = phase.image {
                        image
                            .resizable()
                            .scaledToFill()
                    } else {
                        fallbackContent
                    }
                }
                .frame(width: diameter, height: diameter)
                .clipShape(Circle())
            } else if kind == .team {
                fallbackContent
            } else {
                fallbackContent
            }
        }
        .frame(width: diameter, height: diameter)
        .overlay {
            Circle()
                .stroke(Color.primary.opacity(0.06), lineWidth: 1)
        }
        .accessibilityHidden(true)
    }

    private var decodedImage: UIImage? {
        GaryxDataURLImageCache.image(from: avatarDataUrl)
    }

    private var remoteAvatarURL: URL? {
        let raw = avatarDataUrl.trimmingCharacters(in: .whitespacesAndNewlines)
        guard raw.hasPrefix("http://") || raw.hasPrefix("https://") else { return nil }
        return URL(string: raw)
    }

    @ViewBuilder
    private var fallbackContent: some View {
        if kind == .team {
            Image(systemName: "person.2.fill")
                .font(GaryxFont.system(size: diameter * 0.36, weight: .semibold))
                .foregroundStyle(fallbackForeground)
        } else if let symbol = providerAvatar.symbol {
            Image(systemName: symbol)
                .font(GaryxFont.system(size: providerAvatar.iconSize(for: diameter), weight: .semibold))
                .foregroundStyle(fallbackForeground)
        } else {
            Text(agentInitials)
                .font(GaryxFont.system(size: diameter * 0.32, weight: .bold))
                .foregroundStyle(fallbackForeground)
        }
    }

    private var providerAvatar: GaryxProviderAvatar {
        let source = "\(agentId) \(providerType)".lowercased()
        if source.contains("codex") {
            return .codex
        }
        if source.contains("openai") || source.contains("gpt") {
            return .openAI
        }
        if source.contains("claude") || source.contains("anthropic") {
            return .claude
        }
        if source.contains("gemini") || source.contains("google") {
            return .gemini
        }
        return .generic
    }

    private var agentInitials: String {
        let source = (label.isEmpty ? agentId : label).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !source.isEmpty else { return "A" }
        let words = source
            .replacingOccurrences(of: "(", with: " ")
            .replacingOccurrences(of: ")", with: " ")
            .split { $0 == " " || $0 == "/" || $0 == "_" || $0 == "-" }
        if words.count >= 2, let first = words[0].first, let second = words[1].first {
            return "\(first)\(second)".uppercased()
        }
        return String(source.prefix(2)).uppercased()
    }

    private var fallbackBackground: Color {
        if builtIn, kind == .agent {
            return providerAvatar.background
        }

        let colors = [
            Color(red: 0.88, green: 0.95, blue: 0.90),
            Color(red: 0.90, green: 0.92, blue: 0.98),
            Color(red: 0.96, green: 0.91, blue: 0.84),
            Color(red: 0.91, green: 0.94, blue: 0.96)
        ]
        let seed = (label + agentId).unicodeScalars.reduce(0) { ($0 &+ Int($1.value)) % 997 }
        return colors[seed % colors.count]
    }

    private var fallbackForeground: Color {
        if kind == .team {
            return Color(.systemGray)
        }
        if builtIn {
            return providerAvatar.foreground
        }
        return GaryxTheme.accent
    }
}

struct GaryxAgentIdentityRow: View {
    let id: String
    let title: String
    let subtitle: String
    let kind: GaryxMobileAgentTarget.Kind
    let avatarDataUrl: String
    let providerType: String
    var builtIn: Bool = false
    let selected: Bool

    var body: some View {
        HStack(spacing: 12) {
            GaryxAgentAvatarView(
                agentId: id,
                avatarDataUrl: avatarDataUrl,
                kind: kind,
                label: title,
                providerType: providerType,
                builtIn: builtIn
            )
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(GaryxFont.body(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                if !subtitle.isEmpty {
                    Text(subtitle)
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer()
            if selected {
                Image(systemName: "checkmark.circle.fill")
                    .font(GaryxFont.system(size: 19, weight: .semibold))
                    .foregroundStyle(GaryxTheme.accent)
            }
        }
        .padding(10)
        .contentShape(Rectangle())
    }
}

struct GaryxSelectableRow: View {
    let title: String
    let subtitle: String
    let iconName: String
    let selected: Bool

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: iconName)
                .foregroundStyle(selected ? GaryxTheme.accent : .secondary)
                .frame(width: 28, height: 28)
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(GaryxFont.body(weight: .medium))
                    .foregroundStyle(.primary)
                if !subtitle.isEmpty {
                    Text(subtitle)
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            Spacer()
            if selected {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(GaryxTheme.accent)
            }
        }
        .padding(10)
        .contentShape(Rectangle())
    }
}

struct GaryxInfoRow: View {
    let title: String
    let subtitle: String
    let iconName: String

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: iconName)
                .foregroundStyle(GaryxTheme.accent)
                .frame(width: 28, height: 28)
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(GaryxFont.body(weight: .medium))
                    .foregroundStyle(.primary)
                if !subtitle.isEmpty {
                    Text(subtitle)
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            }
            Spacer()
        }
        .padding(9)
        .contentShape(Rectangle())
    }
}

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
            .font(GaryxFont.system(size: 11, weight: .semibold))
            .foregroundStyle(color)
            .lineLimit(1)
            .fixedSize(horizontal: true, vertical: false)
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(color.opacity(0.10), in: Capsule())
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

struct GaryxNotice: View {
    let title: String
    let text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(title)
                .font(GaryxFont.footnote(weight: .semibold))
                .foregroundStyle(.primary)
            Text(text)
                .font(GaryxFont.callout())
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(10)
        .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .stroke(GaryxTheme.hairline, lineWidth: 1)
        }
    }
}

struct GaryxGlobalErrorToastHost: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let topOffset: CGFloat

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
                .transition(toastTransition)
                .zIndex(100)
            }
        }
        .frame(maxWidth: .infinity, alignment: .top)
        .onAppear {
            present(model.lastError)
        }
        .onChange(of: model.lastError) { _, newValue in
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

    private var toastTransition: AnyTransition {
        if reduceMotion {
            return .opacity
        }
        return .move(edge: .top).combined(with: .opacity)
    }

    private var toastAnimation: Animation? {
        reduceMotion ? nil : .easeOut(duration: 0.18)
    }

    private func present(_ message: String?) {
        let trimmed = message?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !trimmed.isEmpty else {
            toastToken += 1
            withAnimation(toastAnimation) {
                visibleError = nil
            }
            return
        }

        toastToken += 1
        withAnimation(toastAnimation) {
            visibleError = trimmed
        }
    }

    private func hide(message: String) {
        guard visibleError == message else { return }
        toastToken += 1
        withAnimation(toastAnimation) {
            visibleError = nil
        }
        if model.lastError == message {
            model.lastError = nil
        }
    }
}

struct GaryxGlobalErrorToast: View {
    let text: String
    let onDismiss: () -> Void

    var body: some View {
        Button(action: onDismiss) {
            HStack(spacing: 9) {
                Image(systemName: "exclamationmark.circle.fill")
                    .font(GaryxFont.system(size: 14, weight: .semibold))
                    .foregroundStyle(GaryxTheme.danger.opacity(0.86))

                Text(text)
                    .font(GaryxFont.footnote(weight: .medium))
                    .foregroundStyle(.primary)
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
                    .fixedSize(horizontal: false, vertical: true)

                Spacer(minLength: 2)

                Image(systemName: "xmark")
                    .font(GaryxFont.system(size: 10, weight: .bold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .frame(maxWidth: 360, alignment: .leading)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .stroke(GaryxTheme.hairline, lineWidth: 1)
            }
            .shadow(color: Color.black.opacity(0.10), radius: 18, y: 8)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(text)
        .accessibilityHint("Dismiss")
    }
}

struct GaryxEmptyPanelView: View {
    let icon: String
    let title: String
    let text: String

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: icon)
                .font(GaryxFont.title2(weight: .medium))
                .foregroundStyle(.secondary)
            Text(title)
                .font(GaryxFont.body(weight: .semibold))
                .foregroundStyle(.primary)
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
        .padding(.vertical, 36)
    }
}

struct GaryxLoadingPanelView: View {
    let title: String

    var body: some View {
        VStack(spacing: 12) {
            ProgressView()
                .controlSize(.regular)
            Text(title)
                .font(GaryxFont.callout(weight: .medium))
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 24)
        .padding(.vertical, 36)
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

struct GaryxAppLogo: View {
    var size: CGFloat
    var cornerRadius: CGFloat = 22
    var fontSize: CGFloat = 24

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                .fill(Color(.label))

            Text("GX")
                .font(.system(size: fontSize, weight: .semibold, design: .rounded))
                .foregroundStyle(Color(.systemBackground))
        }
        .frame(width: size, height: size)
    }
}

struct GaryxIonicLoader: View {
    var text: String = "Garyx"
    var fontSize: CGFloat = 84
    var isAnimating: Bool = true

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 60.0, paused: !isAnimating)) { context in
            let t = context.date.timeIntervalSinceReferenceDate
            let cycle: Double = 2.6
            let raw = (t / cycle).truncatingRemainder(dividingBy: 1.0)
            let sweep = CGFloat(raw)
            let bob = CGFloat(sin(t * .pi / cycle))

            let baseFont = Font.system(size: fontSize, weight: .black, design: .rounded)

            ZStack {
                Text(text)
                    .font(baseFont)
                    .tracking(-2)
                    .foregroundStyle(
                        AngularGradient(
                            gradient: Gradient(colors: [
                                Color(red: 0.42, green: 0.55, blue: 1.0),
                                Color(red: 0.65, green: 0.40, blue: 1.0),
                                Color(red: 0.30, green: 0.85, blue: 1.0),
                                Color(red: 0.42, green: 0.55, blue: 1.0),
                            ]),
                            center: .center,
                            angle: .degrees(Double(sweep) * 360.0)
                        )
                    )
                    .blur(radius: 26)
                    .opacity(0.55 + Double(abs(bob)) * 0.12)

                Text(text)
                    .font(baseFont)
                    .tracking(-2)
                    .foregroundStyle(
                        LinearGradient(
                            stops: [
                                .init(color: Color(.label).opacity(0.92), location: 0.0),
                                .init(color: Color(.label).opacity(0.55), location: max(0.0, sweep - 0.18)),
                                .init(color: Color(red: 0.55, green: 0.85, blue: 1.0), location: sweep),
                                .init(color: Color(.label).opacity(0.55), location: min(1.0, sweep + 0.18)),
                                .init(color: Color(.label).opacity(0.92), location: 1.0),
                            ],
                            startPoint: .leading,
                            endPoint: .trailing
                        )
                    )
            }
        }
        .accessibilityLabel(Text(text))
    }
}

struct GaryxConnectionPill: View {
    let label: String
    let color: Color
    let isBusy: Bool

    @State private var dotPulse = false

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(color)
                .frame(width: 6, height: 6)
                .scaleEffect(dotPulse ? 1.4 : 1.0)
                .opacity(dotPulse ? 0.6 : 1.0)
                .animation(
                    isBusy
                        ? .easeInOut(duration: 0.8).repeatForever(autoreverses: true)
                        : .default,
                    value: dotPulse
                )

            Text(label)
                .font(GaryxFont.caption(weight: .medium))
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 7)
        .background(Capsule().fill(Color(.systemBackground)))
        .overlay(Capsule().stroke(GaryxTheme.hairline, lineWidth: 1))
        .onAppear {
            dotPulse = isBusy
        }
        .onChange(of: isBusy) { _, newValue in
            dotPulse = newValue
        }
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
                    .font(GaryxFont.system(size: 18, weight: .semibold))
                    .foregroundStyle(.primary)
            } else if let customContent {
                customContent()
            }
        }
        .frame(width: 44, height: 44)
        .garyxAdaptiveGlass(.regular, isInteractive: true, fallbackMaterial: .ultraThinMaterial, in: Circle())
        .contentShape(Rectangle())
    }
}

struct GaryxCompactGlassIcon: View {
    let systemName: String
    var diameter: CGFloat = 32
    var iconSize: CGFloat = 13

    var body: some View {
        Image(systemName: systemName)
            .font(GaryxFont.system(size: iconSize, weight: .medium))
            .foregroundStyle(.primary)
            .frame(width: diameter, height: diameter)
            .garyxAdaptiveGlass(.regular, isInteractive: true, fallbackMaterial: .ultraThinMaterial, in: Circle())
            .contentShape(Rectangle())
    }
}

struct GaryxGlassPanel<Content: View>: View {
    var cornerRadius: CGFloat = 24
    var fallbackMaterial: Material = .ultraThinMaterial
    var tint: Color? = Color(.systemBackground).opacity(0.96)
    var shadowOpacity: Double = 0.055
    private let content: () -> Content

    init(
        cornerRadius: CGFloat = 24,
        fallbackMaterial: Material = .ultraThinMaterial,
        tint: Color? = Color(.systemBackground).opacity(0.96),
        shadowOpacity: Double = 0.055,
        @ViewBuilder content: @escaping () -> Content
    ) {
        self.cornerRadius = cornerRadius
        self.fallbackMaterial = fallbackMaterial
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
                fallbackMaterial: fallbackMaterial,
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
                .font(GaryxFont.system(size: 15, weight: .medium))
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
                        .font(GaryxFont.system(size: 15, weight: .medium))
                        .foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear search")
            }
        }
        .padding(.horizontal, 14)
        .frame(height: 38)
        .garyxAdaptiveGlass(
            .regular,
            isInteractive: true,
            tint: Color(.systemBackground).opacity(0.92),
            fallbackMaterial: .ultraThinMaterial,
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
        .buttonStyle(.plain)
        .accessibilityLabel("Open menu")
    }
}

struct GaryxHeaderMenuIcon: View {
    var body: some View {
        Image(systemName: "line.3.horizontal")
            .font(GaryxFont.system(size: 17, weight: .semibold))
            .foregroundStyle(.primary)
            .frame(width: 44, height: 44)
            .garyxAdaptiveGlass(.regular, isInteractive: true, fallbackMaterial: .ultraThinMaterial, in: Circle())
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
            .font(GaryxFont.system(size: iconSize, weight: iconWeight))
            .foregroundStyle(foreground)
            .frame(width: diameter, height: diameter)
            .background(background, in: Circle())
    }
}

struct GaryxPrimaryCapsuleButton: View {
    let title: String
    var systemImage: String? = nil
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                if let systemImage, !systemImage.isEmpty {
                    Image(systemName: systemImage)
                        .font(GaryxFont.system(size: 15, weight: .semibold))
                }

                Text(title)
                    .font(GaryxFont.body(weight: .semibold))
            }
            .foregroundStyle(Color(.systemBackground))
            .frame(maxWidth: .infinity)
            .frame(height: 56)
            .background(Color(.label), in: Capsule())
        }
        .buttonStyle(.plain)
    }
}

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
    static let warning = Color.orange
    static let danger = Color.red
    static let hairline = Color.primary.opacity(0.08)
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

extension View {
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

private func garyxISO8601Date(from value: String) -> Date? {
    let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return nil }

    let fractional = ISO8601DateFormatter()
    fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    if let date = fractional.date(from: trimmed) {
        return date
    }

    let standard = ISO8601DateFormatter()
    standard.formatOptions = [.withInternetDateTime]
    return standard.date(from: trimmed)
}

extension String {
    var garyxLastPathComponent: String {
        (self as NSString).lastPathComponent
    }

    var garyxDisambiguatedWorkspaceName: String {
        let current = (self as NSString).lastPathComponent
        let parent = ((self as NSString).deletingLastPathComponent as NSString).lastPathComponent
        guard !parent.isEmpty, parent != "/" else {
            return current.isEmpty ? self : current
        }
        return "\(parent)/\(current)"
    }
}
