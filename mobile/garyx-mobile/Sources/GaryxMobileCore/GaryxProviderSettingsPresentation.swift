import Foundation

/// Plans the provider-defaults sheet and the provider-list row: which
/// authentication variant shows, which fields exist, picker/placeholder
/// labels, the echoed draft values, and the save payload. Pure functions of
/// the provider descriptor + model catalog + settings document + draft state,
/// so the SwiftUI layer only binds fields and calls save (TASK-1753).
public enum GaryxProviderSettingsPresentation {

    // MARK: - Section planning

    /// Which authentication section variant the detail sheet shows.
    public enum AuthSection: Equatable, Sendable {
        /// Claude Code: login-entry row driving the OAuth sheet (and the
        /// auth-flow reset when the sheet closes).
        case claudeCode
        /// Native value-typed auth: auth-source menu, optional API key field,
        /// base URL field.
        case native
        /// CLI/OAuth providers: read-only "Managed on the Mac app" row.
        case managedOAuth
    }

    public static func authSection(for provider: GaryxModelProviderDefault) -> AuthSection {
        if provider.providerType == "claude_code" {
            return .claudeCode
        }
        return provider.isNative ? .native : .managedOAuth
    }

    /// gpt is the only provider with a second auth source (the shared Codex
    /// OAuth token); every other native provider has just the API-key source.
    public static func offersGptTokenAuthSource(_ provider: GaryxModelProviderDefault) -> Bool {
        provider.providerType == "gpt"
    }

    /// Service tier (the "Speed" row) is a GPT-only concept and only shows
    /// when the loaded catalog actually supports tier selection.
    public static func supportsServiceTier(
        provider: GaryxModelProviderDefault,
        catalog: GaryxProviderModels?
    ) -> Bool {
        provider.providerType == "gpt" && catalog?.supportsServiceTierSelection == true
    }

    // MARK: - Field-level rules

    /// The auth source a native provider save persists for the current draft:
    /// an empty/whitespace draft falls back to the provider's default source.
    public static func effectiveAuthSource(
        provider: GaryxModelProviderDefault,
        draft: String
    ) -> String {
        let trimmed = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty
            ? GaryxModelProviderDefaults.defaultNativeAuthSource(forProviderType: provider.providerType)
            : trimmed
    }

    public static func authSourceLabel(effectiveAuthSource: String) -> String {
        effectiveAuthSource == "codex" ? "Use GPT token" : "Use API key"
    }

    /// The API-key field hides only while gpt uses the shared token; the
    /// single-source native providers always show it, non-native never do.
    public static func showsApiKeyField(
        provider: GaryxModelProviderDefault,
        effectiveAuthSource: String
    ) -> Bool {
        guard provider.isNative else { return false }
        return provider.providerType != "gpt" || effectiveAuthSource == "api_key"
    }

    public static func apiKeyPlaceholder(for provider: GaryxModelProviderDefault) -> String {
        GaryxModelProviderDefaults.apiKeyEnvName(forProviderType: provider.providerType) ?? "API key"
    }

    /// Switching gpt back to the shared token clears the drafted key, like
    /// the Mac panel; save then blanks a previously stored key. Selecting the
    /// API-key source keeps the draft.
    public static func apiKeyDraft(afterSelectingAuthSource source: String, current: String) -> String {
        source == "codex" ? "" : current
    }

    /// Placeholder for the Model picker: catalog default, then the provider's
    /// static fallback, then the generic label.
    public static func defaultModelLabel(
        provider: GaryxModelProviderDefault,
        catalog: GaryxProviderModels?
    ) -> String {
        let defaultModel = catalog?.defaultModel?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !defaultModel.isEmpty { return "Provider default: \(defaultModel)" }
        let fallback = provider.fallbackDefaultModel.trimmingCharacters(in: .whitespacesAndNewlines)
        if !fallback.isEmpty { return "Provider default: \(fallback)" }
        return "Provider default"
    }

    // MARK: - Draft echo

    /// The values the sheet echoes after hydrating from the authoritative
    /// settings document. Auth fields stay empty for non-native providers
    /// (their auth is host/OAuth-managed and never echoed or saved).
    public struct Draft: Equatable, Sendable {
        public var modelName: String
        public var reasoningEffort: String
        public var serviceTier: String
        public var authSource: String
        public var baseUrl: String
        public var apiKey: String

        public static func make(
            settings: [String: GaryxJSONValue],
            provider: GaryxModelProviderDefault
        ) -> Draft {
            var draft = Draft(
                modelName: GaryxModelProviderDefaults.configuredDefaultModel(in: settings, provider: provider),
                reasoningEffort: GaryxModelProviderDefaults.configuredReasoningEffort(in: settings, provider: provider),
                serviceTier: GaryxModelProviderDefaults.configuredServiceTier(in: settings, provider: provider),
                authSource: "",
                baseUrl: "",
                apiKey: ""
            )
            guard provider.isNative else { return draft }
            draft.authSource = GaryxModelProviderDefaults.configuredAuthSource(in: settings, provider: provider)
            draft.baseUrl = GaryxModelProviderDefaults.configuredBaseUrl(in: settings, provider: provider)
            draft.apiKey = GaryxModelProviderDefaults.configuredApiKey(in: settings, provider: provider)
            return draft
        }
    }

    // MARK: - Save payload

    /// The provider-defaults save payload assembled from the sheet drafts:
    /// service tier only when the provider supports it, auth fields only for
    /// native providers, and the API-key write decided against the
    /// authoritative value echoed at open.
    public struct SaveRequest: Equatable, Sendable {
        public var modelName: String
        public var reasoningEffort: String
        public var serviceTier: String?
        public var authSource: String?
        public var baseUrl: String?
        public var apiKey: GaryxProviderApiKeyUpdate

        public static func make(
            provider: GaryxModelProviderDefault,
            catalog: GaryxProviderModels?,
            modelName: String,
            reasoningEffort: String,
            serviceTier: String,
            authSourceDraft: String,
            baseUrl: String,
            apiKeyDraft: String,
            originalApiKey: String
        ) -> SaveRequest {
            SaveRequest(
                modelName: modelName,
                reasoningEffort: reasoningEffort,
                serviceTier: supportsServiceTier(provider: provider, catalog: catalog) ? serviceTier : nil,
                authSource: provider.isNative
                    ? effectiveAuthSource(provider: provider, draft: authSourceDraft)
                    : nil,
                baseUrl: provider.isNative ? baseUrl : nil,
                apiKey: provider.isNative
                    ? GaryxProviderApiKeyUpdate.make(draft: apiKeyDraft, existing: originalApiKey)
                    : .keep
            )
        }
    }

    // MARK: - Provider list row

    /// The provider-list row's status pill and detail line, derived from the
    /// catalog load state and the configured defaults (effective model chain:
    /// configured → catalog default → static fallback).
    public struct RowModel: Equatable, Sendable {
        public enum Tone: Equatable, Sendable {
            case good
            case muted
            case danger
        }

        public var statusText: String
        public var statusTone: Tone
        public var detailText: String

        public static func make(
            provider: GaryxModelProviderDefault,
            catalog: GaryxProviderModels?,
            settings: [String: GaryxJSONValue]
        ) -> RowModel {
            let error = catalog?.error?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let hasError = !error.isEmpty

            let statusText: String
            let statusTone: Tone
            if hasError {
                statusText = "Error"
                statusTone = .danger
            } else if catalog == nil {
                statusText = "Loading"
                statusTone = .muted
            } else {
                statusText = "Ready"
                statusTone = .good
            }

            var parts: [String] = []
            let configuredModel = GaryxModelProviderDefaults.configuredDefaultModel(in: settings, provider: provider)
            let catalogDefault = catalog?.defaultModel?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let fallbackModel = provider.fallbackDefaultModel.trimmingCharacters(in: .whitespacesAndNewlines)
            let effectiveModel = configuredModel.isEmpty
                ? (catalogDefault.isEmpty ? fallbackModel : catalogDefault)
                : configuredModel
            if !effectiveModel.isEmpty {
                parts.append("Default \(effectiveModel)")
            }
            let configuredReasoning = GaryxModelProviderDefaults.configuredReasoningEffort(in: settings, provider: provider)
            if !configuredReasoning.isEmpty {
                parts.append("Thinking \(configuredReasoning)")
            }
            if catalog?.supportsModelSelection == true {
                parts.append("\(catalog?.models.count ?? 0) models")
            }
            if catalog?.supportsReasoningEffortSelection == true {
                parts.append("\(catalog?.reasoningEfforts.count ?? 0) reasoning")
            }
            if catalog?.supportsServiceTierSelection == true {
                parts.append("\(catalog?.serviceTiers.count ?? 0) tiers")
            }

            let detailText: String
            if parts.isEmpty {
                if hasError {
                    detailText = "Model metadata unavailable"
                } else {
                    detailText = catalog == nil ? "Loading metadata" : "Provider metadata"
                }
            } else {
                detailText = parts.joined(separator: " · ")
            }
            return RowModel(statusText: statusText, statusTone: statusTone, detailText: detailText)
        }
    }
}
