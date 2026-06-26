import Foundation

/// Presentation rules for the per-thread model / thinking-level override chosen
/// while drafting a new thread. The Mac app's composer model control is the
/// source of truth for labels and semantics; mobile adapts only the layout.
public enum GaryxThreadModelOverridePresentation {
    /// Whether the override control should be offered for this provider.
    public static func supportsOverride(_ providerModels: GaryxProviderModels?) -> Bool {
        providerModels?.supportsModelSelection == true
    }

    /// The model that will actually run and should filter thinking levels:
    /// the per-thread override when chosen, else the agent's configured model,
    /// else the provider's default model.
    public static func effortFilterModel(
        override modelOverride: String?,
        agentConfiguredModel: String?,
        providerModels: GaryxProviderModels? = nil
    ) -> String? {
        normalized(modelOverride)
            ?? normalized(agentConfiguredModel)
            ?? normalized(providerModels?.defaultModel)
    }

    /// Thinking levels valid for the current selection: the chosen model's own
    /// list when it constrains efforts, otherwise the provider-level list.
    public static func reasoningEffortOptions(
        providerModels: GaryxProviderModels?,
        model: String?
    ) -> [GaryxProviderModelOption] {
        guard let providerModels, providerModels.supportsReasoningEffortSelection else {
            return []
        }
        if let model = effortScopedModel(providerModels: providerModels, model: model),
           let modelOption = providerModels.models.first(where: { $0.id == model }),
           !modelOption.supportedReasoningEfforts.isEmpty {
            return modelOption.supportedReasoningEfforts
        }
        return providerModels.reasoningEfforts
    }

    /// Default thinking level for a model: model-specific default first, then
    /// the provider-recommended level, then the first advertised level.
    public static func defaultReasoningEffort(
        providerModels: GaryxProviderModels?,
        model: String?
    ) -> String? {
        let explicitModel = normalized(model)
        if let configuredDefault = supportedConfiguredDefaultReasoningEffort(
            providerModels: providerModels,
            model: model
        ) {
            return configuredDefault
        }
        guard let model = explicitModel else {
            return nil
        }
        if let modelOption = providerModels?.models.first(where: { $0.id == model }),
           let effort = normalized(modelOption.defaultReasoningEffort) {
            return effort
        }
        let options = reasoningEffortOptions(providerModels: providerModels, model: model)
        return options.first(where: { $0.recommended }).flatMap { normalized($0.id) }
            ?? options.first.flatMap { normalized($0.id) }
    }

    /// The option id a model / thinking-level picker should mark as selected,
    /// given the value the thread ACTUALLY runs at (`effective`) and the default
    /// for the current model. The empty-id "use default" row is selected when the
    /// effective value is the default; otherwise the effective value's own row is.
    ///
    /// The picker must reflect the effective value — the same value the summary
    /// row shows — not just the per-thread override. Reading the override alone
    /// made the picker fall back to the default row (e.g. "High") while the row
    /// outside showed the real effective level (e.g. "Max").
    public static func selectedOptionId(effective: String?, default defaultValue: String?) -> String {
        guard let effective = normalized(effective) else {
            return ""
        }
        if let defaultValue = normalized(defaultValue), effective == defaultValue {
            return ""
        }
        return effective
    }

    /// Drops a thinking level the current model selection does not support.
    public static func sanitizedReasoningEffort(
        providerModels: GaryxProviderModels?,
        model: String?,
        reasoningEffort: String?
    ) -> String? {
        guard let effort = normalized(reasoningEffort) else {
            return nil
        }
        let options = reasoningEffortOptions(providerModels: providerModels, model: model)
        return options.contains(where: { $0.id == effort }) ? effort : nil
    }

    /// Service tiers valid for the current selection: the chosen model's own
    /// list when it constrains tiers, otherwise the provider-level list.
    public static func serviceTierOptions(
        providerModels: GaryxProviderModels?,
        model: String?
    ) -> [GaryxProviderModelOption] {
        guard let providerModels, providerModels.supportsServiceTierSelection else {
            return []
        }
        if let model = normalized(model),
           let modelOption = providerModels.models.first(where: { $0.id == model }),
           !modelOption.serviceTiers.isEmpty {
            return modelOption.serviceTiers
        }
        return providerModels.serviceTiers
    }

    /// Drops a service tier the current model selection does not support.
    public static func sanitizedServiceTier(
        providerModels: GaryxProviderModels?,
        model: String?,
        serviceTier: String?
    ) -> String? {
        guard let tier = normalized(serviceTier) else {
            return nil
        }
        let options = serviceTierOptions(providerModels: providerModels, model: model)
        return options.contains(where: { $0.id == tier }) ? tier : nil
    }

    public static func serviceTierLabel(
        providerModels: GaryxProviderModels?,
        model: String?,
        serviceTier: String?
    ) -> String? {
        guard let tier = normalized(serviceTier) else {
            return nil
        }
        let options = serviceTierOptions(providerModels: providerModels, model: model)
        return options.first(where: { $0.id == tier })?.label ?? tier
    }

    public static func modelLabel(
        providerModels: GaryxProviderModels?,
        model: String?
    ) -> String? {
        guard let model = normalized(model) else {
            return nil
        }
        return providerModels?.models.first(where: { $0.id == model })?.label ?? model
    }

    public static func reasoningEffortLabel(
        providerModels: GaryxProviderModels?,
        model: String?,
        reasoningEffort: String?
    ) -> String? {
        guard let effort = normalized(reasoningEffort) else {
            return nil
        }
        let options = reasoningEffortOptions(providerModels: providerModels, model: model)
        return options.first(where: { $0.id == effort })?.label ?? effort
    }

    /// Compact label for the new-thread override control.
    public static func controlLabel(
        providerModels: GaryxProviderModels?,
        model: String?,
        reasoningEffort: String?,
        fallback: String
    ) -> String {
        if normalized(model) == nil,
           normalized(reasoningEffort) == nil,
           let defaultModel = normalized(providerModels?.defaultModel),
           let defaultEffort = supportedConfiguredDefaultReasoningEffort(providerModels: providerModels, model: nil) {
            let defaultEffortLabel = reasoningEffortLabel(
                providerModels: providerModels,
                model: defaultModel,
                reasoningEffort: defaultEffort
            ) ?? defaultEffort
            let defaultModelLabel = modelLabel(providerModels: providerModels, model: defaultModel) ?? defaultModel
            return "\(defaultModelLabel) · \(defaultEffortLabel)"
        }
        let modelLabel = modelLabel(providerModels: providerModels, model: model)
        let effortLabel = reasoningEffortLabel(
            providerModels: providerModels,
            model: model,
            reasoningEffort: reasoningEffort
        )
        switch (modelLabel, effortLabel) {
        case let (modelLabel?, effortLabel?):
            return "\(modelLabel) · \(effortLabel)"
        case let (modelLabel?, nil):
            return modelLabel
        case let (nil, effortLabel?):
            return "\(fallback) · \(effortLabel)"
        case (nil, nil):
            return fallback
        }
    }

    private static func normalized(_ value: String?) -> String? {
        guard let value = value?.trimmingCharacters(in: .whitespacesAndNewlines), !value.isEmpty else {
            return nil
        }
        return value
    }

    private static func effortScopedModel(
        providerModels: GaryxProviderModels?,
        model: String?
    ) -> String? {
        normalized(model) ?? normalized(providerModels?.defaultModel)
    }

    private static func supportedConfiguredDefaultReasoningEffort(
        providerModels: GaryxProviderModels?,
        model: String?
    ) -> String? {
        guard let configuredDefault = normalized(providerModels?.defaultReasoningEffort) else {
            return nil
        }
        let options = reasoningEffortOptions(providerModels: providerModels, model: model)
        return options.contains(where: { $0.id == configuredDefault }) ? configuredDefault : nil
    }
}
