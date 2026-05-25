import Foundation
import SwiftUI
import UIKit

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

struct GaryxAgentPickerLabel: View {
    enum Style {
        case prominent
        case compact
        case form
    }

    let target: GaryxMobileAgentTarget?
    let title: String
    let showsChevron: Bool
    var style: Style = .prominent

    var body: some View {
        HStack(spacing: horizontalSpacing) {
            if let target {
                GaryxAgentAvatarView(
                    agentId: target.id,
                    avatarDataUrl: target.avatarDataUrl,
                    kind: target.kind,
                    label: target.title,
                    providerType: target.providerType,
                    builtIn: target.builtIn,
                    diameter: avatarDiameter
                )
            } else {
                Image(systemName: "person.crop.circle")
                    .font(GaryxFont.system(size: fallbackIconSize, weight: .semibold))
                    .foregroundStyle(.secondary)
            }

            Text(title.isEmpty ? "Agent" : title)
                .font(labelFont)
                .foregroundStyle(labelForeground)
                .lineLimit(1)
                .truncationMode(.tail)
                .minimumScaleFactor(0.8)
                .layoutPriority(1)

            if showsChevron {
                Image(systemName: "chevron.down")
                    .font(GaryxFont.system(size: chevronSize, weight: .bold))
                    .foregroundStyle(.tertiary)
            }
        }
        .padding(.horizontal, horizontalPadding)
        .frame(height: labelHeight, alignment: .leading)
        .if(isProminent) { view in
            view.background {
                Capsule()
                    .fill(Color(.systemBackground).opacity(0.42))
                    .background(.ultraThinMaterial, in: Capsule())
            }
        }
        .overlay {
            Capsule()
                .stroke(Color.primary.opacity(isProminent ? 0.03 : 0), lineWidth: 1)
        }
        .contentShape(Capsule())
    }

    private var avatarDiameter: CGFloat {
        switch style {
        case .prominent:
            29
        case .compact:
            16
        case .form:
            24
        }
    }

    private var fallbackIconSize: CGFloat {
        switch style {
        case .prominent:
            22
        case .compact:
            13
        case .form:
            18
        }
    }

    private var chevronSize: CGFloat {
        switch style {
        case .prominent:
            10
        case .compact, .form:
            8
        }
    }

    private var horizontalSpacing: CGFloat {
        switch style {
        case .prominent, .form:
            8
        case .compact:
            6
        }
    }

    private var horizontalPadding: CGFloat {
        switch style {
        case .prominent:
            12
        case .compact, .form:
            0
        }
    }

    private var labelHeight: CGFloat {
        switch style {
        case .prominent:
            44
        case .compact:
            19
        case .form:
            40
        }
    }

    private var labelFont: Font {
        switch style {
        case .prominent:
            GaryxFont.body(weight: .semibold)
        case .compact:
            GaryxFont.caption(weight: .semibold)
        case .form:
            GaryxFont.callout(weight: .medium)
        }
    }

    private var labelForeground: Color {
        switch style {
        case .prominent, .form:
            .primary
        case .compact:
            .secondary
        }
    }

    private var isProminent: Bool {
        switch style {
        case .prominent:
            true
        case .compact, .form:
            false
        }
    }
}

struct GaryxAgentTargetPickerControl: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Binding var selectedAgentTargetId: String
    var style: GaryxAgentPickerLabel.Style = .form
    var showsConfigure = false
    var onConfigure: (() -> Void)?
    @State private var showsPicker = false

    var body: some View {
        Button {
            Task { await model.refreshAgentTargetsIfNeeded() }
            showsPicker = true
        } label: {
            GaryxAgentPickerLabel(
                target: selectedTarget,
                title: selectedLabel,
                showsChevron: true,
                style: style
            )
        }
        .buttonStyle(.plain)
        .popover(
            isPresented: $showsPicker,
            attachmentAnchor: .rect(.bounds),
            arrowEdge: .top
        ) {
            GaryxAgentTargetPickerPopover(
                selectedAgentTargetId: $selectedAgentTargetId,
                showsConfigure: showsConfigure,
                onConfigure: onConfigure
            )
            .environmentObject(model)
            .presentationCompactAdaptation(.popover)
        }
    }

    private var selectedTarget: GaryxMobileAgentTarget? {
        model.agentTargets.first { $0.id == normalizedSelection }
    }

    private var selectedLabel: String {
        selectedTarget?.title ?? (normalizedSelection.isEmpty ? model.agentTargetsPlaceholderText : normalizedSelection)
    }

    private var normalizedSelection: String {
        selectedAgentTargetId.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

struct GaryxAgentTargetPickerPopover: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    @Binding var selectedAgentTargetId: String
    var showsConfigure = false
    var onConfigure: (() -> Void)?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if model.agentTargets.isEmpty {
                Text(model.agentTargetsPlaceholderText)
                    .font(GaryxFont.callout())
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 18)
                    .padding(.vertical, 16)
            } else {
                Text("Latest")
                    .font(GaryxFont.footnote(weight: .semibold))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 20)
                    .padding(.top, 16)
                    .padding(.bottom, 8)

                if model.agentTargets.count <= 5 {
                    ForEach(model.agentTargets) { target in
                        agentRow(for: target)
                    }
                } else {
                    if !agentTargets.isEmpty {
                        ForEach(agentTargets) { target in
                            agentRow(for: target)
                        }
                    }

                    if !teamTargets.isEmpty {
                        Divider()
                            .padding(.horizontal, 20)
                            .padding(.vertical, 8)

                        ForEach(teamTargets) { target in
                            agentRow(for: target)
                        }
                    }
                }
            }

            if showsConfigure {
                Divider()
                    .padding(.horizontal, 18)
                    .padding(.vertical, 8)

                Button {
                    dismiss()
                    onConfigure?()
                } label: {
                    HStack(spacing: 14) {
                        Image(systemName: "slider.horizontal.3")
                            .font(GaryxFont.system(size: 17, weight: .semibold))
                            .frame(width: 30)

                        Text("Configure")
                            .font(GaryxFont.callout(weight: .medium))

                        Spacer(minLength: 0)
                    }
                    .foregroundStyle(.primary)
                    .frame(height: 48)
                    .padding(.horizontal, 20)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
        .frame(width: 308)
        .background(.regularMaterial)
    }

    private var agentTargets: [GaryxMobileAgentTarget] {
        model.agentTargets.filter { $0.kind == .agent }
    }

    private var teamTargets: [GaryxMobileAgentTarget] {
        model.agentTargets.filter { $0.kind == .team }
    }

    private func agentRow(for target: GaryxMobileAgentTarget) -> some View {
        Button {
            selectedAgentTargetId = target.id
            dismiss()
        } label: {
            HStack(spacing: 14) {
                Group {
                    if selectedAgentTargetId == target.id {
                        Image(systemName: "checkmark")
                            .font(GaryxFont.system(size: 18, weight: .semibold))
                            .foregroundStyle(.primary)
                    } else {
                        Color.clear
                    }
                }
                .frame(width: 30)

                GaryxAgentAvatarView(
                    agentId: target.id,
                    avatarDataUrl: target.avatarDataUrl,
                    kind: target.kind,
                    label: target.title,
                    providerType: target.providerType,
                    builtIn: target.builtIn,
                    diameter: 30
                )

                VStack(alignment: .leading, spacing: 2) {
                    Text(target.title)
                        .font(GaryxFont.callout(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)

                    if !target.subtitle.isEmpty {
                        Text(target.subtitle)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }

                Spacer(minLength: 0)
            }
            .frame(height: 54)
            .padding(.horizontal, 20)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
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
