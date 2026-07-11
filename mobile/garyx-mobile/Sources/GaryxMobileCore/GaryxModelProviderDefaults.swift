import Foundation

public struct GaryxModelProviderDefault: Identifiable, Equatable, Sendable {
    public var id: String { providerType }
    public var providerType: String
    public var configKey: String
    public var usageProviderId: String?
    public var fallbackDefaultModel: String

    public init(
        providerType: String,
        configKey: String,
        usageProviderId: String? = nil,
        fallbackDefaultModel: String
    ) {
        self.providerType = providerType
        self.configKey = configKey
        self.usageProviderId = usageProviderId
        self.fallbackDefaultModel = fallbackDefaultModel
    }

}

/// One read-only gateway-host runtime field surfaced on iOS with the
/// "Managed on the Mac app" note (semantic, not security: these are host
/// filesystem concepts that are meaningless to set from a phone).
public struct GaryxProviderHostField: Equatable, Identifiable, Sendable {
    public var id: String { label }
    public var label: String
    public var value: String

    public init(label: String, value: String) {
        self.label = label
        self.value = value
    }
}

public enum GaryxModelProviderDefaults {
    public static let providers: [GaryxModelProviderDefault] = [
        GaryxModelProviderDefault(
            providerType: "claude_code",
            configKey: "claude",
            usageProviderId: "claude_code",
            fallbackDefaultModel: ""
        ),
        GaryxModelProviderDefault(
            providerType: "codex_app_server",
            configKey: "codex",
            usageProviderId: "codex",
            fallbackDefaultModel: ""
        ),
        GaryxModelProviderDefault(
            providerType: "antigravity",
            configKey: "antigravity",
            usageProviderId: "antigravity",
            fallbackDefaultModel: "Claude Opus 4.6 (Thinking)"
        ),
        GaryxModelProviderDefault(providerType: "traex", configKey: "traex", fallbackDefaultModel: ""),
    ]

    public static func provider(for providerType: String) -> GaryxModelProviderDefault? {
        let normalized = providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        return providers.first { $0.providerType == normalized }
    }

    public static func usage(
        in codingUsage: GaryxCodingUsage?,
        provider: GaryxModelProviderDefault
    ) -> GaryxProviderUsage? {
        guard let usageProviderId = provider.usageProviderId else { return nil }
        return codingUsage?.provider(id: usageProviderId)
    }

    public static func providerConfig(
        in settings: [String: GaryxJSONValue],
        provider: GaryxModelProviderDefault
    ) -> [String: GaryxJSONValue] {
        guard case let .object(agents)? = settings["agents"],
              case let .object(config)? = agents[provider.configKey] else {
            return [:]
        }
        return config
    }

    public static func configuredDefaultModel(
        in settings: [String: GaryxJSONValue],
        provider: GaryxModelProviderDefault
    ) -> String {
        stringField("default_model", in: providerConfig(in: settings, provider: provider))
    }

    public static func configuredReasoningEffort(
        in settings: [String: GaryxJSONValue],
        provider: GaryxModelProviderDefault
    ) -> String {
        stringField("model_reasoning_effort", in: providerConfig(in: settings, provider: provider))
    }

    public static func configuredServiceTier(
        in settings: [String: GaryxJSONValue],
        provider: GaryxModelProviderDefault
    ) -> String {
        stringField("model_service_tier", in: providerConfig(in: settings, provider: provider))
    }

    public static func update(
        settings: inout [String: GaryxJSONValue],
        provider: GaryxModelProviderDefault,
        model: String,
        reasoningEffort: String,
        serviceTier: String? = nil
    ) {
        var agents: [String: GaryxJSONValue]
        if case let .object(existingAgents)? = settings["agents"] {
            agents = existingAgents
        } else {
            agents = [:]
        }

        var providerConfig: [String: GaryxJSONValue]
        if case let .object(existingConfig)? = agents[provider.configKey] {
            providerConfig = existingConfig
        } else {
            providerConfig = [:]
        }

        providerConfig["provider_type"] = .string(provider.providerType)
        providerConfig["default_model"] = .string(model.trimmingCharacters(in: .whitespacesAndNewlines))
        providerConfig["model_reasoning_effort"] = .string(reasoningEffort.trimmingCharacters(in: .whitespacesAndNewlines))
        if let serviceTier {
            providerConfig["model_service_tier"] = .string(serviceTier.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        agents[provider.configKey] = .object(providerConfig)
        settings["agents"] = .object(agents)
    }

    /// Read-only gateway-host runtime fields for the iOS detail sheet
    /// ("Managed on the Mac app"). Only the fields that apply to the provider
    /// are listed, only when set.
    public static func hostRuntimeFields(
        in settings: [String: GaryxJSONValue],
        provider: GaryxModelProviderDefault
    ) -> [GaryxProviderHostField] {
        let config = providerConfig(in: settings, provider: provider)
        var fields: [GaryxProviderHostField] = []

        func append(_ label: String, _ key: String) {
            let value = stringField(key, in: config)
            if !value.isEmpty {
                fields.append(GaryxProviderHostField(label: label, value: value))
            }
        }

        switch provider.providerType {
        case "claude_code":
            append("CLI mode", "claude_cli_mode")
            append("CLI path", "claude_cli_path")
            append("Permission mode", "permission_mode")
        case "codex_app_server":
            append("Codex home", "codex_home")
        case "antigravity":
            append("Antigravity binary", "antigravity_bin")
        default:
            break
        }

        if case let .object(env)? = config["env"] {
            let lines = env.keys.sorted().compactMap { key -> String? in
                guard case let .string(value)? = env[key] else { return nil }
                return "\(key)=\(value)"
            }
            if !lines.isEmpty {
                fields.append(GaryxProviderHostField(label: "Environment", value: lines.joined(separator: "\n")))
            }
        }
        return fields
    }

    private static func stringField(_ key: String, in object: [String: GaryxJSONValue]) -> String {
        guard case let .string(value)? = object[key] else { return "" }
        return value.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
