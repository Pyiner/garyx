import Foundation
import SwiftUI

// Model-provider settings surfaces: the provider list (with the shared §4
// usage visualization inline) and the provider detail sheet with sectioned
// editing. Business rules (patch shape, env-key map, usage display models)
// live in GaryxMobileCore; these views dumb-render Core models.

struct GaryxSettingsProviderContent: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var selectedProvider: GaryxModelProviderDefault?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxSectionBlock(title: "Model Providers") {
                GaryxCompactListGroup {
                    let providers = GaryxModelProviderDefaults.providers
                    ForEach(Array(providers.enumerated()), id: \.element.id) { index, provider in
                        Button {
                            selectedProvider = provider
                            Task { await model.loadProviderModels(providerType: provider.providerType) }
                        } label: {
                            GaryxProviderModelsRow(
                                provider: provider,
                                catalog: model.providerModelsByType[provider.providerType],
                                settings: model.gatewaySettingsDocument,
                                usage: GaryxModelProviderDefaults.usage(
                                    in: model.codingUsage,
                                    provider: provider
                                ),
                                usageRefreshedAt: model.codingUsage?.refreshedAt
                            )
                        }
                        .buttonStyle(.plain)

                        if index < providers.count - 1 {
                            GaryxCompactRowDivider()
                        }
                    }
                }
            }
        }
        .task {
            await model.refreshCodingUsageWidget()
            for provider in GaryxModelProviderDefaults.providers
            where model.providerModelsByType[provider.providerType] == nil {
                await model.loadProviderModels(providerType: provider.providerType)
            }
        }
        .fullScreenCover(item: $selectedProvider) { provider in
            GaryxModelProviderDefaultsSheet(provider: provider)
        }
    }
}

struct GaryxProviderModelsRow: View {
    let provider: GaryxModelProviderDefault
    let catalog: GaryxProviderModels?
    let settings: [String: GaryxJSONValue]
    let usage: GaryxProviderUsage?
    var usageRefreshedAt: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(spacing: 9) {
                Image(systemName: iconName)
                    .font(GaryxFont.system(size: 14, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 20, height: 20)

                VStack(alignment: .leading, spacing: 2) {
                    Text(providerPresentation.displayName)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(detail)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                Spacer(minLength: 8)

                GaryxStatusPill(text: statusText, tone: statusTone)
                Image(systemName: "chevron.right")
                    .font(GaryxFont.system(size: 11, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }

            GaryxProviderUsageInlineBlock(
                provider: provider,
                usageDisplay: usageDisplay
            )
            .padding(.leading, 29)
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 7)
        .contentShape(Rectangle())
    }

    private var iconName: String {
        providerPresentation.symbolName ?? "cpu"
    }

    private var providerPresentation: GaryxProviderPresentation {
        GaryxProviderPresentation.make(providerType: provider.providerType)
    }

    private var usageDisplay: GaryxProviderUsageDisplayModel? {
        GaryxProviderUsageDisplayModel.make(from: usage, refreshedAt: usageRefreshedAt)
    }

    private var hasError: Bool {
        let error = catalog?.error?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return !error.isEmpty
    }

    private var statusText: String {
        if hasError { return "Error" }
        return catalog == nil ? "Loading" : "Ready"
    }

    private var statusTone: GaryxStatusPill.Tone {
        if hasError { return .danger }
        return catalog == nil ? .muted : .good
    }

    private var detail: String {
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
        if parts.isEmpty {
            if hasError {
                return "Model metadata unavailable"
            }
            return catalog == nil ? "Loading metadata" : "Provider metadata"
        }
        return parts.joined(separator: " · ")
    }
}

// MARK: - Shared §4 usage visualization

extension GaryxUsageLevel {
    /// Shared remaining-% severity tint (D3): `≥50 green / 20–50 amber / <20
    /// red`, muted when unavailable. The thresholds live in Core
    /// (`GaryxUsageGaugeModel.level`); this is the single tint mapping.
    var garyxTint: Color {
        switch self {
        case .healthy:
            return GaryxTheme.accent
        case .warning:
            return GaryxTheme.warning
        case .critical:
            return GaryxTheme.danger
        case .unavailable:
            return Color.secondary
        }
    }
}

/// One labelled remaining-quota meter: `label ····· 73%` over a fill track
/// with a `resets in 2d 4h` caption. Used inline in the provider list row and
/// in the detail sheet's Usage section.
private struct GaryxUsageMeterRow: View {
    let label: String
    let remainingPercent: Double
    let remainingText: String
    let caption: String
    let level: GaryxUsageLevel
    var compact = false

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(label)
                    .font(GaryxFont.caption(weight: compact ? .regular : .medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.72)
                Spacer(minLength: 6)
                Text(remainingText)
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                if !caption.isEmpty {
                    Text(caption)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                        .minimumScaleFactor(0.7)
                }
            }
            GeometryReader { proxy in
                ZStack(alignment: .leading) {
                    Capsule()
                        .fill(Color.primary.opacity(0.08))
                    Capsule()
                        .fill(level.garyxTint)
                        .frame(width: max(0, proxy.size.width * remainingPercent / 100))
                }
            }
            .frame(height: compact ? 4 : 5)
        }
    }
}

private struct GaryxUsagePillsRow: View {
    let display: GaryxProviderUsageDisplayModel
    var showsUpdated = false

    var body: some View {
        if display.plan != nil || display.stale || (showsUpdated && display.updatedText != nil) {
            HStack(spacing: 6) {
                if let plan = display.plan {
                    GaryxStatusPill(text: plan, tone: .good)
                }
                if display.stale {
                    GaryxStatusPill(text: "stale", tone: .warning)
                }
                if showsUpdated, let updated = display.updatedText {
                    Text(updated)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                }
            }
        }
    }
}

/// The inline usage block under a provider list row: plan/stale pills plus
/// Session+Weekly meters (Claude/Codex) or per-model mini-bars (Antigravity),
/// dimmed when stale; the five non-metered providers show "No quota data"
/// rather than hiding (D6).
private struct GaryxProviderUsageInlineBlock: View {
    let provider: GaryxModelProviderDefault
    let usageDisplay: GaryxProviderUsageDisplayModel?

    var body: some View {
        if provider.usageProviderId == nil {
            usageCaption("No quota data")
        } else if let usageDisplay {
            if !usageDisplay.available {
                usageCaption(usageDisplay.summaryText)
            } else {
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 6) {
                        Text("Usage")
                            .font(GaryxFont.caption(weight: .semibold))
                            .foregroundStyle(.secondary)
                        GaryxUsagePillsRow(display: usageDisplay)
                    }
                    meters(usageDisplay)
                }
                .opacity(usageDisplay.stale ? 0.55 : 1)
            }
        } else {
            usageCaption("Loading")
        }
    }

    @ViewBuilder
    private func meters(_ display: GaryxProviderUsageDisplayModel) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            ForEach(display.windows) { window in
                GaryxUsageMeterRow(
                    label: window.label,
                    remainingPercent: window.remainingPercent,
                    remainingText: window.remainingText,
                    caption: window.detailText,
                    level: window.level,
                    compact: true
                )
            }
            ForEach(display.models) { model in
                GaryxUsageMeterRow(
                    label: model.title,
                    remainingPercent: model.remainingPercent,
                    remainingText: model.remainingText,
                    caption: model.detailText,
                    level: model.level,
                    compact: true
                )
            }
        }
    }

    private func usageCaption(_ text: String) -> some View {
        HStack(spacing: 6) {
            Text("Usage")
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(.secondary)
            Text(text)
                .font(GaryxFont.caption())
                .foregroundStyle(.secondary)
        }
    }
}

// MARK: - Provider detail sheet

struct GaryxModelProviderDefaultsSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let provider: GaryxModelProviderDefault
    @State private var modelName = ""
    @State private var reasoningEffort = ""
    @State private var serviceTier = ""
    @State private var authSource = ""
    @State private var baseUrl = ""
    @State private var apiKey = ""
    /// The authoritative key echoed at open; drives set/blank/keep on save.
    @State private var originalApiKey = ""
    @State private var isHydrated = false
    @State private var isSaving = false

    var body: some View {
        GaryxFormSheet(
            title: "\(providerPresentation.displayName) Defaults",
            canSave: isHydrated && !isSaving,
            onSave: saveDefaults
        ) {
            VStack(alignment: .leading, spacing: 22) {
                GaryxFormGroupedSection(title: "Provider") {
                    GaryxFormReadOnlyRow(title: "Name", value: providerPresentation.displayName)
                    Divider().padding(.leading, 16)
                    GaryxFormReadOnlyRow(title: "Type", value: provider.providerType)
                }

                if provider.usageProviderId != nil {
                    GaryxProviderUsageFormSection(
                        usageDisplay: GaryxProviderUsageDisplayModel.make(
                            from: GaryxModelProviderDefaults.usage(in: model.codingUsage, provider: provider),
                            refreshedAt: model.codingUsage?.refreshedAt
                        )
                    )
                }

                GaryxFormGroupedSection(title: "Defaults") {
                    GaryxProviderDefaultPickerRow(
                        title: "Model",
                        value: $modelName,
                        placeholder: defaultModelLabel,
                        options: modelOptions,
                        iconName: "cpu"
                    )
                    if !reasoningOptions.isEmpty {
                        Divider().padding(.leading, 16)
                        GaryxProviderDefaultPickerRow(
                            title: "Thinking level",
                            value: $reasoningEffort,
                            placeholder: "Provider default",
                            options: reasoningOptions,
                            iconName: "brain"
                        )
                    }
                    if supportsServiceTier {
                        Divider().padding(.leading, 16)
                        GaryxProviderDefaultPickerRow(
                            title: "Speed",
                            value: $serviceTier,
                            placeholder: "Standard",
                            options: serviceTierOptions,
                            iconName: "gauge.with.needle",
                            emptyOptionLabel: "Standard"
                        )
                    }
                }

                authenticationSection

                hostRuntimeSection
            }
        }
        .task {
            async let catalogLoad: Void = model.loadProviderModels(providerType: provider.providerType)
            _ = await model.refreshAuthoritativeGatewaySettings()
            _ = await catalogLoad
            fillDraft()
            isHydrated = true
        }
        .onChange(of: modelName) { _, _ in
            reasoningEffort = GaryxThreadModelOverridePresentation.sanitizedReasoningEffort(
                providerModels: catalog,
                model: modelName,
                reasoningEffort: reasoningEffort
            ) ?? ""
        }
    }

    // MARK: Sections

    @ViewBuilder
    private var authenticationSection: some View {
        if provider.isNative {
            VStack(alignment: .leading, spacing: 6) {
                GaryxFormGroupedSection(title: "Authentication") {
                    if provider.providerType == "gpt" {
                        GaryxFormMenuRow(title: "Auth", value: authSourceLabel) {
                            Button("Use GPT token") { selectAuthSource("codex") }
                            Button("Use API key") { selectAuthSource("api_key") }
                        }
                    }
                    if showsApiKeyField {
                        if provider.providerType == "gpt" {
                            Divider().padding(.leading, 16)
                        }
                        GaryxFormTextFieldRow(
                            title: "API key",
                            text: $apiKey,
                            placeholder: apiKeyPlaceholder,
                            keyboardType: .asciiCapable,
                            autocapitalization: .never,
                            autocorrectionDisabled: true
                        )
                    }
                    if provider.providerType == "gpt" || showsApiKeyField {
                        Divider().padding(.leading, 16)
                    }
                    GaryxFormTextFieldRow(
                        title: "Base URL",
                        text: $baseUrl,
                        placeholder: "Provider default",
                        keyboardType: .URL,
                        autocapitalization: .never,
                        autocorrectionDisabled: true,
                        wrapsValue: true
                    )
                }
                Text("The API key is stored on the gateway as \(apiKeyPlaceholder) and shown here in plain text. Clearing it blanks the key; remove it fully from the Mac app.")
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 14)
                    .fixedSize(horizontal: false, vertical: true)
            }
        } else {
            GaryxFormGroupedSection(title: "Authentication") {
                GaryxFormReadOnlyRow(title: "OAuth", value: "Managed on the Mac app")
            }
        }
    }

    @ViewBuilder
    private var hostRuntimeSection: some View {
        let fields = GaryxModelProviderDefaults.hostRuntimeFields(
            in: model.gatewaySettingsDocument,
            provider: provider
        )
        if !fields.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                GaryxFormGroupedSection(title: "CLI Runtime") {
                    ForEach(Array(fields.enumerated()), id: \.element.id) { index, field in
                        if index > 0 {
                            Divider().padding(.leading, 16)
                        }
                        if field.value.contains("\n") {
                            GaryxFormReadOnlyMultilineRow(
                                title: field.label,
                                value: field.value,
                                valuePlacement: .below
                            )
                        } else {
                            GaryxFormReadOnlyRow(title: field.label, value: field.value)
                        }
                    }
                }
                Text("Gateway-host runtime settings. Managed on the Mac app.")
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 14)
            }
        }
    }

    // MARK: State

    private var providerPresentation: GaryxProviderPresentation {
        GaryxProviderPresentation.make(providerType: provider.providerType)
    }

    private var catalog: GaryxProviderModels? {
        model.providerModelsByType[provider.providerType]
    }

    private var modelOptions: [GaryxProviderModelOption] {
        catalog?.models ?? []
    }

    private var reasoningOptions: [GaryxProviderModelOption] {
        GaryxThreadModelOverridePresentation.reasoningEffortOptions(
            providerModels: catalog,
            model: modelName
        )
    }

    private var supportsServiceTier: Bool {
        provider.providerType == "gpt" && catalog?.supportsServiceTierSelection == true
    }

    private var serviceTierOptions: [GaryxProviderModelOption] {
        catalog?.serviceTiers ?? []
    }

    private var effectiveAuthSource: String {
        let trimmed = authSource.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty
            ? GaryxModelProviderDefaults.defaultNativeAuthSource(forProviderType: provider.providerType)
            : trimmed
    }

    private var authSourceLabel: String {
        effectiveAuthSource == "codex" ? "Use GPT token" : "Use API key"
    }

    private var showsApiKeyField: Bool {
        guard provider.isNative else { return false }
        return provider.providerType != "gpt" || effectiveAuthSource == "api_key"
    }

    private var apiKeyPlaceholder: String {
        GaryxModelProviderDefaults.apiKeyEnvName(forProviderType: provider.providerType) ?? "API key"
    }

    private var defaultModelLabel: String {
        let defaultModel = catalog?.defaultModel?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !defaultModel.isEmpty { return "Provider default: \(defaultModel)" }
        let fallback = provider.fallbackDefaultModel.trimmingCharacters(in: .whitespacesAndNewlines)
        if !fallback.isEmpty { return "Provider default: \(fallback)" }
        return "Provider default"
    }

    private func selectAuthSource(_ source: String) {
        authSource = source
        // Switching GPT back to the shared token clears the draft key, like Mac;
        // save then blanks a previously stored key.
        if source == "codex" {
            apiKey = ""
        }
    }

    private func fillDraft() {
        let settings = model.gatewaySettingsDocument
        modelName = GaryxModelProviderDefaults.configuredDefaultModel(in: settings, provider: provider)
        reasoningEffort = GaryxModelProviderDefaults.configuredReasoningEffort(in: settings, provider: provider)
        serviceTier = GaryxModelProviderDefaults.configuredServiceTier(in: settings, provider: provider)
        guard provider.isNative else { return }
        authSource = GaryxModelProviderDefaults.configuredAuthSource(in: settings, provider: provider)
        baseUrl = GaryxModelProviderDefaults.configuredBaseUrl(in: settings, provider: provider)
        apiKey = GaryxModelProviderDefaults.configuredApiKey(in: settings, provider: provider)
        originalApiKey = apiKey
    }

    private func saveDefaults() {
        guard !isSaving, isHydrated else { return }
        isSaving = true
        Task {
            let didSave = await model.updateModelProviderDefaults(
                provider: provider,
                modelName: modelName,
                reasoningEffort: reasoningEffort,
                serviceTier: supportsServiceTier ? serviceTier : nil,
                authSource: provider.isNative ? effectiveAuthSource : nil,
                baseUrl: provider.isNative ? baseUrl : nil,
                apiKey: provider.isNative
                    ? GaryxProviderApiKeyUpdate.make(draft: apiKey, existing: originalApiKey)
                    : .keep
            )
            await MainActor.run {
                isSaving = false
                if didSave {
                    dismiss()
                }
            }
        }
    }
}

/// The detail sheet's Usage section: full §4 treatment — plan pill, stale tag,
/// freshness line, Session/Weekly meters or all Antigravity buckets.
private struct GaryxProviderUsageFormSection: View {
    let usageDisplay: GaryxProviderUsageDisplayModel?

    var body: some View {
        GaryxFormGroupedSection(title: "Usage") {
            VStack(alignment: .leading, spacing: 10) {
                if let usageDisplay {
                    GaryxUsagePillsRow(display: usageDisplay, showsUpdated: true)
                    if !usageDisplay.available {
                        Text(usageDisplay.summaryText)
                            .font(GaryxFont.body())
                            .foregroundStyle(.secondary)
                        Text(usageDisplay.detailText)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.tertiary)
                    } else if usageDisplay.windows.isEmpty && usageDisplay.models.isEmpty {
                        Text("No quota data")
                            .font(GaryxFont.body())
                            .foregroundStyle(.secondary)
                    } else {
                        VStack(alignment: .leading, spacing: 9) {
                            ForEach(usageDisplay.windows) { window in
                                GaryxUsageMeterRow(
                                    label: window.label,
                                    remainingPercent: window.remainingPercent,
                                    remainingText: window.remainingText,
                                    caption: window.detailText,
                                    level: window.level
                                )
                            }
                            ForEach(usageDisplay.models) { modelRow in
                                GaryxUsageMeterRow(
                                    label: modelRow.title,
                                    remainingPercent: modelRow.remainingPercent,
                                    remainingText: modelRow.remainingText,
                                    caption: modelRow.detailText,
                                    level: modelRow.level
                                )
                            }
                        }
                        .opacity(usageDisplay.stale ? 0.55 : 1)
                    }
                } else {
                    Text("No quota data")
                        .font(GaryxFont.body())
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 14)
        }
    }
}

private struct GaryxProviderDefaultPickerRow: View {
    let title: String
    @Binding var value: String
    let placeholder: String
    let options: [GaryxProviderModelOption]
    let iconName: String
    var emptyOptionLabel = "Provider default"

    var body: some View {
        GaryxFormMenuRow(title: title) {
            Button(emptyOptionLabel) {
                value = ""
            }
            if !options.isEmpty {
                Divider()
            }
            ForEach(options, id: \.id) { option in
                Button(optionTitle(option)) {
                    value = option.id
                }
            }
        } valueLabel: {
            HStack(spacing: 6) {
                Text(selectedLabel)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Image(systemName: "chevron.up.chevron.down")
                    .font(GaryxFont.system(size: 10, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            .foregroundStyle(.primary)
        }
        .disabled(options.isEmpty && normalizedValue.isEmpty)
    }

    private var normalizedValue: String {
        value.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var selectedLabel: String {
        guard !normalizedValue.isEmpty else { return placeholder }
        return options.first(where: { $0.id == normalizedValue })?.label ?? normalizedValue
    }

    private func optionTitle(_ option: GaryxProviderModelOption) -> String {
        option.recommended ? "\(option.label) · Recommended" : option.label
    }
}
