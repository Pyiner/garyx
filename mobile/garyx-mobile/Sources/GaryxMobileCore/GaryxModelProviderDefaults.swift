import Foundation

public struct GaryxModelProviderDefault: Identifiable, Equatable, Sendable {
    public var id: String { providerType }
    public var providerType: String
    public var configKey: String
    public var fallbackDefaultModel: String

    public init(providerType: String, configKey: String, fallbackDefaultModel: String) {
        self.providerType = providerType
        self.configKey = configKey
        self.fallbackDefaultModel = fallbackDefaultModel
    }
}

public enum GaryxModelProviderDefaults {
    public static let providers: [GaryxModelProviderDefault] = [
        GaryxModelProviderDefault(providerType: "claude_code", configKey: "claude", fallbackDefaultModel: ""),
        GaryxModelProviderDefault(providerType: "codex_app_server", configKey: "codex", fallbackDefaultModel: ""),
        GaryxModelProviderDefault(providerType: "traex", configKey: "traex", fallbackDefaultModel: ""),
        GaryxModelProviderDefault(providerType: "gemini_cli", configKey: "gemini", fallbackDefaultModel: "gemini-3-flash-preview"),
        GaryxModelProviderDefault(providerType: "gpt", configKey: "gpt", fallbackDefaultModel: "gpt-5.5"),
        GaryxModelProviderDefault(providerType: "anthropic", configKey: "anthropic", fallbackDefaultModel: "claude-sonnet-4-6"),
        GaryxModelProviderDefault(providerType: "google", configKey: "google", fallbackDefaultModel: "gemini-3-flash-preview")
    ]

    public static func provider(for providerType: String) -> GaryxModelProviderDefault? {
        let normalized = providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        return providers.first { $0.providerType == normalized }
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

    public static func update(
        settings: inout [String: GaryxJSONValue],
        provider: GaryxModelProviderDefault,
        model: String,
        reasoningEffort: String
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
        agents[provider.configKey] = .object(providerConfig)
        settings["agents"] = .object(agents)
    }

    private static func stringField(_ key: String, in object: [String: GaryxJSONValue]) -> String {
        guard case let .string(value)? = object[key] else { return "" }
        return value.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
