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
        /// CLI/OAuth providers: read-only "Managed on the Mac app" row.
        case managedOAuth
    }

    public static func authSection(for provider: GaryxModelProviderDefault) -> AuthSection {
        if provider.providerType == "claude_code" {
            return .claudeCode
        }
        return .managedOAuth
    }

    /// Service tier (the "Speed" row) only shows when the loaded catalog
    /// advertises tier selection.
    public static func supportsServiceTier(
        provider: GaryxModelProviderDefault,
        catalog: GaryxProviderModels?
    ) -> Bool {
        catalog?.supportsServiceTierSelection == true
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
    /// settings document.
    public struct Draft: Equatable, Sendable {
        public var modelName: String
        public var reasoningEffort: String
        public var serviceTier: String

        public static func make(
            settings: [String: GaryxJSONValue],
            provider: GaryxModelProviderDefault
        ) -> Draft {
            Draft(
                modelName: GaryxModelProviderDefaults.configuredDefaultModel(in: settings, provider: provider),
                reasoningEffort: GaryxModelProviderDefaults.configuredReasoningEffort(in: settings, provider: provider),
                serviceTier: GaryxModelProviderDefaults.configuredServiceTier(in: settings, provider: provider)
            )
        }
    }

    // MARK: - Save payload

    /// The provider-defaults save payload assembled from the sheet drafts:
    /// service tier only when the provider supports it.
    public struct SaveRequest: Equatable, Sendable {
        public var modelName: String
        public var reasoningEffort: String
        public var serviceTier: String?

        public static func make(
            provider: GaryxModelProviderDefault,
            catalog: GaryxProviderModels?,
            modelName: String,
            reasoningEffort: String,
            serviceTier: String
        ) -> SaveRequest {
            SaveRequest(
                modelName: modelName,
                reasoningEffort: reasoningEffort,
                serviceTier: supportsServiceTier(provider: provider, catalog: catalog) ? serviceTier : nil
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
