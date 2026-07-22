import Foundation

public enum GaryxProviderIdentityKind: String, Equatable {
    case antigravity
    case codex
    case traex
    case claude
    case generic
}

public struct GaryxProviderFallbackRGB: Equatable, Sendable {
    public let red: Double
    public let green: Double
    public let blue: Double

    public init(red: Double, green: Double, blue: Double) {
        self.red = red
        self.green = green
        self.blue = blue
    }
}

public struct GaryxProviderPresentation: Equatable {
    public let kind: GaryxProviderIdentityKind
    public let displayName: String
    public let fallbackAssetName: String?
    public let symbolName: String?
    public let fallbackInitials: String
    public let fallbackBackgroundRGB: GaryxProviderFallbackRGB
    public let iconSizeFactor: Double
    public let prefersLightFallbackForeground: Bool

    public static func make(providerType: String) -> GaryxProviderPresentation {
        let normalized = providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        let kind = kind(for: normalized)
        return GaryxProviderPresentation(
            kind: kind,
            displayName: displayName(for: normalized, kind: kind),
            fallbackAssetName: fallbackAssetName(for: kind),
            symbolName: symbolName(for: kind),
            fallbackInitials: initials(for: displayName(for: normalized, kind: kind), fallback: "P"),
            fallbackBackgroundRGB: fallbackBackgroundRGB(for: kind),
            iconSizeFactor: iconSizeFactor(for: kind),
            prefersLightFallbackForeground: prefersLightFallbackForeground(for: kind)
        )
    }

    public static func make(
        agentId: String?,
        providerType: String?,
        fallbackName: String? = nil
    ) -> GaryxProviderPresentation {
        let provider = providerType?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let agent = agentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let kind = kind(for: provider.isEmpty ? agent : provider)
        let label = fallbackName?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let display = provider.isEmpty
            ? (label.isEmpty ? (agent.isEmpty ? "Agent" : agent) : label)
            : displayName(for: provider, kind: kind)
        return GaryxProviderPresentation(
            kind: kind,
            displayName: display,
            fallbackAssetName: fallbackAssetName(for: kind),
            symbolName: symbolName(for: kind),
            fallbackInitials: initials(for: label.isEmpty ? display : label, fallback: "A"),
            fallbackBackgroundRGB: fallbackBackgroundRGB(for: kind),
            iconSizeFactor: iconSizeFactor(for: kind),
            prefersLightFallbackForeground: prefersLightFallbackForeground(for: kind)
        )
    }

    public static func displayName(for providerType: String) -> String {
        let kind = kind(for: providerType)
        return displayName(for: providerType, kind: kind)
    }

    public static func initials(for value: String, fallback: String) -> String {
        let source = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !source.isEmpty else { return fallback }
        let words = source
            .replacingOccurrences(of: "(", with: " ")
            .replacingOccurrences(of: ")", with: " ")
            .split { $0 == " " || $0 == "/" || $0 == "_" || $0 == "-" }
        if words.count >= 2, let first = words[0].first, let second = words[1].first {
            return "\(first)\(second)".uppercased()
        }
        return String(source.prefix(2)).uppercased()
    }

    private static func kind(for value: String) -> GaryxProviderIdentityKind {
        let source = value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if source.contains("antigravity")
            || source == "agy"
            || source.hasPrefix("agy_")
            || source.hasPrefix("agy-") {
            return .antigravity
        }
        if source.contains("codex") {
            return .codex
        }
        // TRAE CLI is a Codex fork; match before the generic fallback. Its
        // identifiers ("traex"/"trae") never contain "codex", so order is safe.
        if source.contains("traex") || source.contains("trae") {
            return .traex
        }
        if source.contains("claude") {
            return .claude
        }
        return .generic
    }

    private static func symbolName(for kind: GaryxProviderIdentityKind) -> String? {
        switch kind {
        case .antigravity:
            "bolt.fill"
        case .codex:
            "chevron.left.forwardslash.chevron.right"
        case .traex:
            nil
        case .claude:
            "sparkles"
        case .generic:
            nil
        }
    }

    private static func fallbackAssetName(for kind: GaryxProviderIdentityKind) -> String? {
        switch kind {
        case .antigravity:
            "ProviderAntigravity"
        case .codex:
            "ProviderCodex"
        case .traex:
            "ProviderTrae"
        case .claude:
            "ProviderClaude"
        case .generic:
            nil
        }
    }

    private static func fallbackBackgroundRGB(for kind: GaryxProviderIdentityKind) -> GaryxProviderFallbackRGB {
        switch kind {
        case .antigravity:
            GaryxProviderFallbackRGB(red: 0.15, green: 0.36, blue: 0.30)
        case .codex, .traex:
            GaryxProviderFallbackRGB(red: 0.08, green: 0.10, blue: 0.12)
        case .claude:
            GaryxProviderFallbackRGB(red: 0.50, green: 0.37, blue: 0.26)
        case .generic:
            GaryxProviderFallbackRGB(red: 0.95, green: 0.95, blue: 0.97)
        }
    }

    private static func iconSizeFactor(for kind: GaryxProviderIdentityKind) -> Double {
        switch kind {
        case .antigravity:
            0.36
        case .codex, .traex:
            0.32
        case .claude:
            0.40
        case .generic:
            0.36
        }
    }

    private static func prefersLightFallbackForeground(for kind: GaryxProviderIdentityKind) -> Bool {
        kind != .generic
    }

    private static func displayName(for providerType: String, kind: GaryxProviderIdentityKind) -> String {
        let normalized = providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        switch kind {
        case .antigravity:
            return "Antigravity"
        case .codex:
            return "Codex"
        case .traex:
            return "Traex"
        case .claude:
            return "Claude Code"
        case .generic:
            let words = normalized
                .replacingOccurrences(of: "_", with: " ")
                .replacingOccurrences(of: "-", with: " ")
                .split(separator: " ")
                .map { word in
                    word.prefix(1).uppercased() + word.dropFirst()
                }
            if !words.isEmpty {
                return words.joined(separator: " ")
            }
            return "Provider"
        }
    }
}

public struct GaryxChannelIdentityPresentation: Equatable {
    public let channel: String
    public let displayName: String
    public let fallbackAssetName: String?
    public let fallbackInitials: String

    public static func make(channel: String, label: String? = nil) -> GaryxChannelIdentityPresentation {
        let normalized = channel.trimmingCharacters(in: .whitespacesAndNewlines)
        let displayName = displayName(for: normalized)
        let label = label?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return GaryxChannelIdentityPresentation(
            channel: normalized,
            displayName: displayName,
            fallbackAssetName: fallbackAssetName(for: normalized),
            fallbackInitials: initials(for: label.isEmpty ? displayName : label)
        )
    }

    public static func displayName(for channel: String, catalogDisplayName: String?) -> String {
        let catalogDisplayName = catalogDisplayName?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !catalogDisplayName.isEmpty {
            return catalogDisplayName
        }
        return displayName(for: channel)
    }

    public static func displayName(for channel: String) -> String {
        let normalized = channel.trimmingCharacters(in: .whitespacesAndNewlines)
        switch normalized.lowercased() {
        case "telegram":
            return "Telegram"
        case "feishu":
            return "Feishu"
        case "weixin":
            return "Weixin"
        case "discord":
            return "Discord"
        case "api":
            return "API"
        default:
            return normalized.isEmpty
                ? "Channel"
                : normalized.replacingOccurrences(of: "_", with: " ").capitalized
        }
    }

    private static func fallbackAssetName(for channel: String) -> String? {
        switch channel.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "telegram":
            return "ChannelTelegram"
        case "discord":
            return "ChannelDiscord"
        case "feishu":
            return "ChannelFeishu"
        case "weixin":
            return "ChannelWeixin"
        default:
            return nil
        }
    }

    private static func initials(for value: String) -> String {
        let words = value
            .replacingOccurrences(of: "_", with: " ")
            .replacingOccurrences(of: "-", with: " ")
            .split(separator: " ")
        let initials = words.prefix(2).compactMap { $0.first }.map { String($0).uppercased() }.joined()
        return initials.isEmpty ? "B" : initials
    }
}
