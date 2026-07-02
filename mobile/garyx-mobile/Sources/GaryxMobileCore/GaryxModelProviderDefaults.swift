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

    /// Native model-loop providers expose value-typed auth editing (API key,
    /// base URL, auth source) on every surface; the CLI providers stay
    /// OAuth/host-managed.
    public var isNative: Bool {
        GaryxModelProviderDefaults.nativeProviderTypes.contains(providerType)
    }
}

/// How a provider-defaults save should treat the API key stored at
/// `agents.<key>.env[<ENV_NAME>]`. `PUT /api/settings?merge=true` deep-merges
/// objects and cannot delete keys, so clearing writes an empty string (blank =
/// "no key" for native auth); hard removal stays a Mac-app action.
public enum GaryxProviderApiKeyUpdate: Equatable, Sendable {
    case keep
    case set(String)
    case blank

    /// Decide the write from the editor draft plus the authoritative value that
    /// was echoed when the editor opened: a non-empty draft sets, an emptied
    /// draft blanks only when a key actually existed, and an untouched empty
    /// field never creates an empty env entry.
    public static func make(draft: String, existing: String) -> GaryxProviderApiKeyUpdate {
        let trimmed = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty { return .set(trimmed) }
        let existingTrimmed = existing.trimmingCharacters(in: .whitespacesAndNewlines)
        return existingTrimmed.isEmpty ? .keep : .blank
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
        GaryxModelProviderDefault(providerType: "gemini_cli", configKey: "gemini", fallbackDefaultModel: "gemini-3-flash-preview"),
        GaryxModelProviderDefault(providerType: "gpt", configKey: "gpt", fallbackDefaultModel: "gpt-5.5"),
        GaryxModelProviderDefault(providerType: "anthropic", configKey: "anthropic", fallbackDefaultModel: "claude-sonnet-4-6"),
        GaryxModelProviderDefault(providerType: "google", configKey: "google", fallbackDefaultModel: "gemini-3-flash-preview")
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

    /// Providers whose value-typed auth fields (API key / base URL / auth
    /// source) are editable on every surface. Mirrors the Mac panel's
    /// `group === 'native'` rows.
    public static let nativeProviderTypes: Set<String> = ["gpt", "anthropic", "google"]

    /// The env var each native provider stores its API key under — the same
    /// map as the Mac panel's `apiKeyEnvName`. API keys live at
    /// `agents.<key>.env[<ENV_NAME>]`, never a dedicated config field.
    public static func apiKeyEnvName(forProviderType providerType: String) -> String? {
        switch providerType.trimmingCharacters(in: .whitespacesAndNewlines) {
        case "gpt":
            return "OPENAI_API_KEY"
        case "anthropic":
            return "ANTHROPIC_API_KEY"
        case "google":
            return "GEMINI_API_KEY"
        default:
            return nil
        }
    }

    /// Mirrors the Mac panel's `defaultNativeAuthSource`: GPT shares the Codex
    /// OAuth token by default; the other native providers use an API key.
    public static func defaultNativeAuthSource(forProviderType providerType: String) -> String {
        providerType.trimmingCharacters(in: .whitespacesAndNewlines) == "gpt" ? "codex" : "api_key"
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

    public static func configuredAuthSource(
        in settings: [String: GaryxJSONValue],
        provider: GaryxModelProviderDefault
    ) -> String {
        stringField("auth_source", in: providerConfig(in: settings, provider: provider))
    }

    public static func configuredBaseUrl(
        in settings: [String: GaryxJSONValue],
        provider: GaryxModelProviderDefault
    ) -> String {
        stringField("base_url", in: providerConfig(in: settings, provider: provider))
    }

    /// The plaintext API key echoed into the editor (D1), read from
    /// `agents.<key>.env[<ENV_NAME>]`.
    public static func configuredApiKey(
        in settings: [String: GaryxJSONValue],
        provider: GaryxModelProviderDefault
    ) -> String {
        guard let envName = apiKeyEnvName(forProviderType: provider.providerType),
              case let .object(env)? = providerConfig(in: settings, provider: provider)["env"],
              case let .string(value)? = env[envName] else {
            return ""
        }
        return value.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    public static func update(
        settings: inout [String: GaryxJSONValue],
        provider: GaryxModelProviderDefault,
        model: String,
        reasoningEffort: String,
        serviceTier: String? = nil,
        authSource: String? = nil,
        baseUrl: String? = nil,
        apiKey: GaryxProviderApiKeyUpdate = .keep
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
        // Service tier is a GPT-only field, mirroring the Mac save path; an
        // empty string clears back to the provider default (deep-merge cannot
        // delete scalar keys, and empty reads as unset everywhere).
        if let serviceTier, provider.providerType == "gpt" {
            providerConfig["model_service_tier"] = .string(serviceTier.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        if provider.isNative {
            if let authSource {
                let trimmed = authSource.trimmingCharacters(in: .whitespacesAndNewlines)
                providerConfig["auth_source"] = .string(
                    trimmed.isEmpty ? defaultNativeAuthSource(forProviderType: provider.providerType) : trimmed
                )
            }
            if let baseUrl {
                providerConfig["base_url"] = .string(baseUrl.trimmingCharacters(in: .whitespacesAndNewlines))
            }
            if let envName = apiKeyEnvName(forProviderType: provider.providerType) {
                switch apiKey {
                case .keep:
                    break
                case let .set(value):
                    mergeEnvValue(value.trimmingCharacters(in: .whitespacesAndNewlines), forKey: envName, into: &providerConfig)
                case .blank:
                    mergeEnvValue("", forKey: envName, into: &providerConfig)
                }
            }
        }
        agents[provider.configKey] = .object(providerConfig)
        settings["agents"] = .object(agents)
    }

    /// Read-only gateway-host runtime fields for the iOS detail sheet
    /// ("Managed on the Mac app"). Only the fields that apply to the provider
    /// are listed, only when set; the free-form env mirror excludes the API-key
    /// env var, which the Authentication section owns.
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
        case "gemini_cli":
            append("Gemini binary", "gemini_bin")
            append("Approval mode", "approval_mode")
        case "antigravity":
            append("Antigravity binary", "antigravity_bin")
        case "gpt":
            append("Codex home", "codex_home")
        default:
            break
        }

        if case let .object(env)? = config["env"] {
            let apiKeyEnv = apiKeyEnvName(forProviderType: provider.providerType)
            let lines = env.keys.sorted().compactMap { key -> String? in
                guard key != apiKeyEnv, case let .string(value)? = env[key] else { return nil }
                return "\(key)=\(value)"
            }
            if !lines.isEmpty {
                fields.append(GaryxProviderHostField(label: "Environment", value: lines.joined(separator: "\n")))
            }
        }
        return fields
    }

    private static func mergeEnvValue(
        _ value: String,
        forKey envName: String,
        into providerConfig: inout [String: GaryxJSONValue]
    ) {
        var env: [String: GaryxJSONValue]
        if case let .object(existingEnv)? = providerConfig["env"] {
            env = existingEnv
        } else {
            env = [:]
        }
        env[envName] = .string(value)
        providerConfig["env"] = .object(env)
    }

    private static func stringField(_ key: String, in object: [String: GaryxJSONValue]) -> String {
        guard case let .string(value)? = object[key] else { return "" }
        return value.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
