import Foundation
import SwiftUI
import UIKit
import WidgetKit

// Model-provider settings surfaces: the provider list (topped by the Quota
// hero, with the shared §4 usage visualization inline) and the provider
// detail sheet with sectioned editing. Business rules (patch shape and usage
// display models) live in GaryxMobileCore; these views dumb-render Core models.

struct GaryxSettingsProviderContent: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var selectedProvider: GaryxModelProviderDefault?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxSectionBlock(title: "Quota") {
                GaryxCompactListGroup {
                    GaryxProviderQuotaHero(usage: model.codingUsage)
                }
            }
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
                        .buttonStyle(GaryxPressableRowStyle())

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
        .garyxFullScreenCover(item: $selectedProvider) { provider in
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
                    .font(GaryxFont.fixedSystem(size: 14, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 20, height: 20)

                VStack(alignment: .leading, spacing: 2) {
                    Text(providerPresentation.displayName)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .garyxReadingLineLimit()
                    Text(rowModel.detailText)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .garyxReadingLineLimit()
                }

                Spacer(minLength: 8)

                GaryxStatusPill(text: rowModel.statusText, tone: rowModel.statusTone.garyxPillTone)
                Image(systemName: "chevron.right")
                    .font(GaryxFont.fixedSystem(size: 11, weight: .semibold))
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

    private var rowModel: GaryxProviderSettingsPresentation.RowModel {
        .make(provider: provider, catalog: catalog, settings: settings)
    }
}

private extension GaryxProviderSettingsPresentation.RowModel.Tone {
    /// The single Core-tone → pill-tone mapping (the tone semantics live in
    /// Core; the pill type is view-layer).
    var garyxPillTone: GaryxStatusPill.Tone {
        switch self {
        case .good:
            return .good
        case .muted:
            return .muted
        case .danger:
            return .danger
        }
    }
}

// MARK: - Quota hero (design §6.4/D8)

private extension GaryxCodingUsageMetrics {
    /// Hero sizing: the widget's medium-family gauge visual, tightened so
    /// three columns fit the provider page width.
    static var providerQuotaHero: GaryxCodingUsageMetrics {
        var metrics = GaryxCodingUsageMetrics(family: .systemMedium)
        metrics.gaugeSpacing = 14
        metrics.gaugeLineWidth = 9
        metrics.gaugeValueSize = 22
        metrics.gaugeIconSize = 11
        metrics.gaugeLabelSpacing = 4
        metrics.gaugeMaxWidth = 88
        metrics.titleSize = 12
        metrics.detailSize = 10
        return metrics
    }
}

/// The widget deep-link landing at the top of the provider list: a horizontal
/// row of quota gauges for the three metered providers, rendering the shared
/// widget speedometer from Core hero models. Stale gauges dim and surface the
/// "updated Nm ago" freshness caption (design §4).
private struct GaryxProviderQuotaHero: View {
    let usage: GaryxCodingUsage?

    private var gauges: [GaryxUsageGaugeModel] {
        GaryxUsageGaugeModel.heroModels(from: usage)
    }

    private var staleUpdatedText: String? {
        guard gauges.contains(where: \.stale) else { return nil }
        return GaryxUsageGaugeModel.usageUpdatedText(refreshedAt: usage?.refreshedAt)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .top, spacing: GaryxCodingUsageMetrics.providerQuotaHero.gaugeSpacing) {
                ForEach(gauges, id: \.providerId) { gauge in
                    GaryxUsageSpeedometer(model: gauge, metrics: .providerQuotaHero)
                        .opacity(gauge.stale ? 0.55 : 1)
                }
            }
            if let staleUpdatedText {
                Text(staleUpdatedText)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.tertiary)
                    .frame(maxWidth: .infinity, alignment: .center)
            }
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 10)
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
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @ScaledMetric(relativeTo: .caption) private var readingSpacing: CGFloat = 3
    @ScaledMetric(relativeTo: .caption) private var trackScale: CGFloat = 1
    let label: String
    let remainingPercent: Double
    let remainingText: String
    let caption: String
    let level: GaryxUsageLevel
    var compact = false

    var body: some View {
        VStack(alignment: .leading, spacing: readingSpacing) {
            if dynamicTypeSize.garyxUsesExpandedReadingLayout {
                VStack(alignment: .leading, spacing: readingSpacing) {
                    meterLabel
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        remainingLabel
                        captionLabel
                    }
                }
            } else {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    meterLabel
                    Spacer(minLength: 6)
                    remainingLabel
                    captionLabel
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
            .frame(height: (compact ? 4 : 5) * trackScale)
        }
    }

    private var meterLabel: some View {
        Text(label)
            .font(GaryxFont.caption(weight: compact ? .regular : .medium))
            .foregroundStyle(.secondary)
            .garyxReadingLineLimit()
    }

    private var remainingLabel: some View {
        Text(remainingText)
            .font(GaryxFont.caption(weight: .semibold))
            .foregroundStyle(.primary)
            .garyxReadingLineLimit()
    }

    @ViewBuilder
    private var captionLabel: some View {
        if !caption.isEmpty {
            Text(caption)
                .font(GaryxFont.caption())
                .foregroundStyle(.tertiary)
                .garyxReadingLineLimit()
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
                        .garyxReadingLineLimit()
                }
            }
        }
    }
}

/// The inline usage block under a provider list row: plan/stale pills plus
/// Session/weekly/scoped meters (Claude/Codex) or per-model mini-bars (Antigravity),
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
                        // Stale readings must show their freshness (§4): the
                        // updated-ago caption joins the stale tag inline.
                        GaryxUsagePillsRow(display: usageDisplay, showsUpdated: usageDisplay.stale)
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
    @State private var showsClaudeLoginSheet = false
    @State private var isHydrated = false
    @State private var hydrationFailed = false
    @State private var isSaving = false

    var body: some View {
        GaryxFormSheet(
            title: "\(providerPresentation.displayName) Defaults",
            canSave: isHydrated && !isSaving,
            onCancel: closeSheet,
            onSave: saveDefaults
        ) {
            Group {
                if hydrationFailed {
                    Section {
                        GaryxFormErrorText(text: "Couldn't load the current provider settings from the gateway, so editing is disabled to avoid overwriting newer values.")
                        Button {
                            Task { await hydrate() }
                        } label: {
                            Text("Retry")
                                .fontWeight(.semibold)
                                .frame(maxWidth: .infinity)
                        }
                    }
                }

                GaryxFormGroupedSection(title: "Provider") {
                    GaryxFormReadOnlyRow(title: "Name", value: providerPresentation.displayName)
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
                        GaryxProviderDefaultPickerRow(
                            title: "Thinking level",
                            value: $reasoningEffort,
                            placeholder: "Provider default",
                            options: reasoningOptions,
                            iconName: "brain"
                        )
                    }
                    if supportsServiceTier {
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
        .task { await hydrate() }
        .garyxSheet(isPresented: $showsClaudeLoginSheet) {
            GaryxClaudeCodeLoginSheet()
        }
        .onDisappear {
            if authSection == .claudeCode {
                model.resetClaudeCodeAuthFlow()
            }
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
        switch authSection {
        case .claudeCode:
            GaryxClaudeCodeAuthEntryRow(entry: claudeCodeAuthEntry) {
                showsClaudeLoginSheet = true
            }
        case .managedOAuth:
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
            Section {
                ForEach(fields) { field in
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
            } header: {
                Text("CLI Runtime")
                    .textCase(nil)
            } footer: {
                Text("Gateway-host runtime settings. Managed on the Mac app.")
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

    private var authSection: GaryxProviderSettingsPresentation.AuthSection {
        GaryxProviderSettingsPresentation.authSection(for: provider)
    }

    private var supportsServiceTier: Bool {
        GaryxProviderSettingsPresentation.supportsServiceTier(provider: provider, catalog: catalog)
    }

    private var serviceTierOptions: [GaryxProviderModelOption] {
        catalog?.serviceTiers ?? []
    }

    private var claudeCodeAuthEntry: GaryxClaudeCodeAuthEntry {
        GaryxClaudeCodeAuthEntry.make(
            session: model.claudeCodeAuthSession,
            usage: GaryxModelProviderDefaults.usage(in: model.codingUsage, provider: provider)
        )
    }

    private var defaultModelLabel: String {
        GaryxProviderSettingsPresentation.defaultModelLabel(provider: provider, catalog: catalog)
    }

    /// Loads the authoritative settings document before echoing values (D1 /
    /// §6.2). Hydration gates editing: on failure nothing is echoed and Save
    /// stays disabled so a stale restored projection can never be written back.
    private func hydrate() async {
        async let catalogLoad: Void = model.loadProviderModels(providerType: provider.providerType)
        let fetched = await model.refreshAuthoritativeGatewaySettings()
        _ = await catalogLoad
        if fetched {
            fillDraft()
            isHydrated = true
            hydrationFailed = false
        } else if !isHydrated {
            hydrationFailed = true
        }
    }

    private func fillDraft() {
        let draft = GaryxProviderSettingsPresentation.Draft.make(
            settings: model.gatewaySettingsDocument,
            provider: provider
        )
        modelName = draft.modelName
        reasoningEffort = draft.reasoningEffort
        serviceTier = draft.serviceTier
    }

    private func saveDefaults() {
        guard !isSaving, isHydrated else { return }
        isSaving = true
        Task {
            let didSave = await model.updateModelProviderDefaults(
                provider: provider,
                request: GaryxProviderSettingsPresentation.SaveRequest.make(
                    provider: provider,
                    catalog: catalog,
                    modelName: modelName,
                    reasoningEffort: reasoningEffort,
                    serviceTier: serviceTier
                )
            )
            await MainActor.run {
                isSaving = false
                if didSave {
                    dismiss()
                }
            }
        }
    }

    private func closeSheet() {
        if authSection == .claudeCode {
            model.resetClaudeCodeAuthFlow()
        }
        dismiss()
    }
}

/// The detail sheet's Usage section: full §4 treatment — plan pill, stale tag,
/// freshness line, session/weekly/scoped meters, or all Antigravity buckets.
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
            .padding(.vertical, 4)
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
                    .garyxReadingLineLimit()
                    .truncationMode(.middle)
                Image(systemName: "chevron.up.chevron.down")
                    .font(GaryxFont.fixedSystem(size: 10, weight: .semibold))
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
