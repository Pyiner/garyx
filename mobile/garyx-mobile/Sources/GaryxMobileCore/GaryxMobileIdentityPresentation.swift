import Foundation

public enum GaryxProviderIdentityKind: String, Equatable {
    case codex
    case openAI
    case claude
    case gemini
    case generic
}

public struct GaryxProviderPresentation: Equatable {
    public let kind: GaryxProviderIdentityKind
    public let displayName: String
    public let symbolName: String?
    public let fallbackInitials: String

    public static func make(providerType: String) -> GaryxProviderPresentation {
        let normalized = providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        let kind = kind(for: normalized)
        return GaryxProviderPresentation(
            kind: kind,
            displayName: displayName(for: normalized, kind: kind),
            symbolName: symbolName(for: kind),
            fallbackInitials: initials(for: displayName(for: normalized, kind: kind), fallback: "P")
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
            symbolName: symbolName(for: kind),
            fallbackInitials: initials(for: label.isEmpty ? display : label, fallback: "A")
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

    private static func symbolName(for kind: GaryxProviderIdentityKind) -> String? {
        switch kind {
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

    private static func displayName(for providerType: String, kind: GaryxProviderIdentityKind) -> String {
        let normalized = providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        switch normalized {
        case "codex_app_server":
            return "Codex"
        case "claude_code":
            return "Claude Code"
        case "gemini_cli":
            return "Gemini CLI"
        case "gpt":
            return "OpenAI"
        case "anthropic", "claude_llm":
            return "Anthropic"
        case "google", "gemini_llm":
            return "Google"
        default:
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
            switch kind {
            case .codex:
                return "Codex"
            case .openAI:
                return "OpenAI"
            case .claude:
                return "Claude"
            case .gemini:
                return "Gemini"
            case .generic:
                return "Provider"
            }
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
