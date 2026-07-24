import Foundation

/// One row in a runtime-settings picker. The empty id is the follow-default row
/// (clears the thread's cell); any other id pins exactly that value.
public struct GaryxRuntimePickerOption: Equatable, Identifiable, Sendable {
    public let id: String
    public let label: String

    public init(id: String, label: String) {
        self.id = id
        self.label = label
    }
}

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

    /// The row a runtime-settings picker marks as selected. The thread's own
    /// cell is the truth: a pinned value marks its own row, an empty cell marks
    /// the follow-default row.
    ///
    /// This must not resolve the effective value. A cell pinned to the value
    /// that also happens to be the current default would then mark the
    /// follow-default row and read as "not pinned", and the pinned row would
    /// show no checkmark at all. The follow-default row carries a fixed label
    /// (never a concrete value's label), so the summary row outside can keep
    /// showing the effective value without contradicting the checkmark.
    public static func selectedPickerOptionId(cell: String?) -> String {
        normalized(cell) ?? ""
    }

    /// Rows for the per-thread model picker.
    public static func modelPickerOptions(
        providerModels: GaryxProviderModels?,
        effectiveModel: String?,
        defaultRowLabel: String
    ) -> [GaryxRuntimePickerOption] {
        pickerOptions(
            advertised: providerModels?.models ?? [],
            current: effectiveModel,
            defaultRowLabel: defaultRowLabel
        ) { value in
            modelLabel(providerModels: providerModels, model: value)
        }
    }

    /// Rows for the per-thread thinking-level picker, scoped to the model that
    /// will actually run.
    public static func reasoningEffortPickerOptions(
        providerModels: GaryxProviderModels?,
        model: String?,
        effectiveReasoningEffort: String?,
        defaultRowLabel: String
    ) -> [GaryxRuntimePickerOption] {
        pickerOptions(
            advertised: reasoningEffortOptions(providerModels: providerModels, model: model),
            current: effectiveReasoningEffort,
            defaultRowLabel: defaultRowLabel
        ) { value in
            reasoningEffortLabel(
                providerModels: providerModels,
                model: model,
                reasoningEffort: value
            )
        }
    }

    /// Rows for the per-thread service-tier picker, scoped to the model that
    /// will actually run.
    public static func serviceTierPickerOptions(
        providerModels: GaryxProviderModels?,
        model: String?,
        effectiveServiceTier: String?,
        defaultRowLabel: String
    ) -> [GaryxRuntimePickerOption] {
        pickerOptions(
            advertised: serviceTierOptions(providerModels: providerModels, model: model),
            current: effectiveServiceTier,
            defaultRowLabel: defaultRowLabel
        ) { value in
            serviceTierLabel(
                providerModels: providerModels,
                model: model,
                serviceTier: value
            )
        }
    }

    /// Shared row contract for every runtime-settings picker:
    ///
    /// 1. Row 0 is always the follow-default row (empty id) carrying the given
    ///    fixed label. It never borrows a concrete value's label: a row reading
    ///    "Claude Opus 5" is expected to pin Opus 5, not to clear the cell.
    /// 2. Every advertised option keeps its own real-id row, including the one
    ///    that happens to be the current default. Suppressing that row left the
    ///    default's label reachable only through the empty-id row, so choosing
    ///    it cleared the cell and the thread silently fell back to the bound
    ///    agent's model instead.
    /// 3. A current value the provider does not advertise is appended last so it
    ///    stays visible and re-selectable.
    ///
    /// Advertised ids are normalized before use: a blank id would otherwise
    /// produce a second row that the server trims back to the empty string, so
    /// a concrete-looking row would silently clear the cell. Rows exist only
    /// when at least one advertised option survives normalization — a picker
    /// offering only "follow default" has nothing to choose.
    private static func pickerOptions(
        advertised: [GaryxProviderModelOption],
        current: String?,
        defaultRowLabel: String,
        label: (String) -> String?
    ) -> [GaryxRuntimePickerOption] {
        var seen = Set<String>([""])
        var advertisedOptions: [GaryxRuntimePickerOption] = []
        for option in advertised {
            guard let id = normalized(option.id), seen.insert(id).inserted else {
                continue
            }
            advertisedOptions.append(GaryxRuntimePickerOption(id: id, label: option.label))
        }
        guard !advertisedOptions.isEmpty else {
            return []
        }
        var options = [GaryxRuntimePickerOption(id: "", label: defaultRowLabel)]
        options.append(contentsOf: advertisedOptions)
        if let current = normalized(current), seen.insert(current).inserted {
            options.append(
                GaryxRuntimePickerOption(id: current, label: label(current) ?? current)
            )
        }
        return options
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
