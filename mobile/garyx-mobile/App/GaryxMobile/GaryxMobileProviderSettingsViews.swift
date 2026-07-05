import Foundation
import SwiftUI
import UIKit
import WidgetKit

// Model-provider settings surfaces: the provider list (topped by the Quota
// hero, with the shared §4 usage visualization inline) and the provider
// detail sheet with sectioned editing. Business rules (patch shape, env-key
// map, usage display models) live in GaryxMobileCore; these views dumb-render
// Core models.

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
    @Environment(\.openURL) private var openURL
    @EnvironmentObject private var model: GaryxMobileModel
    let provider: GaryxModelProviderDefault
    @State private var modelName = ""
    @State private var reasoningEffort = ""
    @State private var serviceTier = ""
    @State private var claudeAuthMode = GaryxClaudeCodeAuthMode.claudeai
    @State private var claudeAuthUsesSSO = false
    @State private var claudeAuthEmail = ""
    @State private var claudeAuthCode = ""
    @State private var authSource = ""
    @State private var baseUrl = ""
    @State private var apiKey = ""
    /// The authoritative key echoed at open; drives set/blank/keep on save.
    @State private var originalApiKey = ""
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
            VStack(alignment: .leading, spacing: 22) {
                if hydrationFailed {
                    VStack(alignment: .leading, spacing: 10) {
                        GaryxFormErrorText(text: "Couldn't load the current provider settings from the gateway, so editing is disabled to avoid overwriting newer values.")
                        Button {
                            Task { await hydrate() }
                        } label: {
                            Text("Retry")
                                .font(GaryxFont.body(weight: .medium))
                                .padding(.horizontal, 14)
                        }
                        .buttonStyle(.bordered)
                        .padding(.horizontal, 14)
                    }
                }

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
        .task { await hydrate() }
        .onDisappear {
            if provider.providerType == "claude_code" {
                model.resetClaudeCodeAuthFlow()
            }
        }
        .onChange(of: model.claudeCodeAuthSession?.loginId) { _, _ in
            claudeAuthCode = ""
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
        if provider.providerType == "claude_code" {
            GaryxClaudeCodeAuthSection(
                presentation: claudeCodeAuthPresentation,
                mode: $claudeAuthMode,
                usesSSO: $claudeAuthUsesSSO,
                email: $claudeAuthEmail,
                authorizationCode: $claudeAuthCode,
                onPrimaryAction: performClaudeCodeAuthPrimaryAction,
                onPasteCode: pasteClaudeAuthCode,
                onSubmitCode: submitClaudeAuthCode
            )
        } else if provider.isNative {
            VStack(alignment: .leading, spacing: 6) {
                GaryxFormGroupedSection(title: "Authentication") {
                    // Every native provider gets the auth-source row (D1). Only
                    // GPT has a second source (the shared Codex OAuth token);
                    // Anthropic/Google expose their single API-key source so the
                    // saved auth_source is always visible, never written silently.
                    GaryxFormMenuRow(title: "Auth", value: authSourceLabel) {
                        if provider.providerType == "gpt" {
                            Button("Use GPT token") { selectAuthSource("codex") }
                        }
                        Button("Use API key") { selectAuthSource("api_key") }
                    }
                    if showsApiKeyField {
                        Divider().padding(.leading, 16)
                        GaryxFormTextFieldRow(
                            title: "API key",
                            text: $apiKey,
                            placeholder: apiKeyPlaceholder,
                            keyboardType: .asciiCapable,
                            autocapitalization: .never,
                            autocorrectionDisabled: true
                        )
                    }
                    Divider().padding(.leading, 16)
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

    private var claudeCodeAuthPresentation: GaryxClaudeCodeAuthPresentation {
        GaryxClaudeCodeAuthPresentation.make(
            session: model.claudeCodeAuthSession,
            usage: GaryxModelProviderDefaults.usage(in: model.codingUsage, provider: provider),
            authorizationCode: claudeAuthCode
        )
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

    private func closeSheet() {
        if provider.providerType == "claude_code" {
            model.resetClaudeCodeAuthFlow()
        }
        dismiss()
    }

    private func performClaudeCodeAuthPrimaryAction() {
        switch claudeCodeAuthPresentation.primaryAction {
        case .start:
            Task {
                if let url = await model.startClaudeCodeAuth(
                    mode: claudeAuthMode,
                    sso: claudeAuthUsesSSO,
                    email: claudeAuthEmail
                ) {
                    openURL(url)
                }
            }
        case .openAuthorizationURL:
            if let url = model.claudeCodeAuthSession?.authorizationURL {
                openURL(url)
            }
        case .none:
            break
        }
    }

    private func pasteClaudeAuthCode() {
        claudeAuthCode = UIPasteboard.general.string ?? ""
    }

    private func submitClaudeAuthCode() {
        Task {
            await model.submitClaudeCodeAuth(code: claudeAuthCode)
        }
    }
}

private struct GaryxClaudeCodeAuthSection: View {
    let presentation: GaryxClaudeCodeAuthPresentation
    @Binding var mode: GaryxClaudeCodeAuthMode
    @Binding var usesSSO: Bool
    @Binding var email: String
    @Binding var authorizationCode: String
    let onPrimaryAction: () -> Void
    let onPasteCode: () -> Void
    let onSubmitCode: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            GaryxFormGroupedSection(title: "Authentication") {
                GaryxFormRow(title: "Status") {
                    GaryxStatusPill(
                        text: presentation.statusText,
                        tone: presentation.tone.garyxStatusPillTone
                    )
                }
                Divider().padding(.leading, 16)
                GaryxFormReadOnlyRow(title: "Account", value: accountValue)
                Divider().padding(.leading, 16)
                GaryxFormRow(title: "Action") {
                    Button(presentation.primaryActionTitle, action: onPrimaryAction)
                        .buttonStyle(.bordered)
                        .disabled(!presentation.primaryActionEnabled)
                }
                if presentation.showsLoginOptions {
                    Divider().padding(.leading, 16)
                    GaryxFormMenuRow(title: "Login method", value: mode.displayName) {
                        ForEach(GaryxClaudeCodeAuthMode.allCases) { option in
                            Button(option.displayName) {
                                mode = option
                            }
                        }
                    }
                    Divider().padding(.leading, 16)
                    GaryxFormRow(title: "Use SSO") {
                        Toggle("", isOn: $usesSSO)
                            .labelsHidden()
                    }
                    Divider().padding(.leading, 16)
                    GaryxFormTextFieldRow(
                        title: "Email",
                        text: $email,
                        placeholder: "Optional",
                        keyboardType: .emailAddress,
                        textContentType: .emailAddress,
                        autocapitalization: .never,
                        autocorrectionDisabled: true
                    )
                }
                if presentation.showsCodeField {
                    Divider().padding(.leading, 16)
                    authorizationCodeRow
                    Divider().padding(.leading, 16)
                    GaryxFormRow(title: "Submit") {
                        Button("Submit code", action: onSubmitCode)
                            .buttonStyle(.borderedProminent)
                            .disabled(!presentation.submitEnabled)
                    }
                }
            }
            if let detail = presentation.detailText {
                Text(detail)
                    .font(GaryxFont.caption())
                    .foregroundStyle(presentation.tone == .danger ? GaryxTheme.danger : .secondary)
                    .padding(.horizontal, 14)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let accountDetail = presentation.accountDetailText {
                Text(accountDetail)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 14)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var authorizationCodeRow: some View {
        GaryxFormRow(title: "Authorization code") {
            HStack(spacing: 8) {
                TextField("Paste code", text: $authorizationCode)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled(true)
                    .keyboardType(.asciiCapable)
                    .lineLimit(1)
                Button("Paste", action: onPasteCode)
                    .font(GaryxFont.callout(weight: .medium))
                    .buttonStyle(.bordered)
            }
        }
    }

    private var accountValue: String {
        if let account = presentation.accountText {
            return account
        }
        return presentation.statusText == "Signed in" ? "Gateway host" : "Not signed in"
    }
}

private extension GaryxClaudeCodeAuthPresentationTone {
    var garyxStatusPillTone: GaryxStatusPill.Tone {
        switch self {
        case .good:
            return .good
        case .warning:
            return .warning
        case .danger:
            return .danger
        case .muted:
            return .muted
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
