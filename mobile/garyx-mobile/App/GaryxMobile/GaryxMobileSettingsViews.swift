import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct GaryxMobileSettingsPanel: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsGatewaySetup = false
    @State private var showsCreateBot = false
    @State private var showsCreateCommand = false
    @State private var showsCreateMcp = false

    var body: some View {
        GaryxPanelScaffold(
            title: settingsTitle,
            subtitle: model.activeSettingsTab.label,
            onRefresh: { await model.connectAndRefresh() },
            leadingActionLabel: settingsLeadingActionLabel,
            leadingActionSystemName: "chevron.left",
            leadingAction: settingsLeadingAction,
            background: GaryxTheme.background
        ) {
            VStack(alignment: .leading, spacing: 12) {
                GaryxSettingsTabContent()
            }
        } actions: {
            HStack(spacing: 8) {
                switch model.activeSettingsTab {
                case .gateway:
                    GaryxAddToolbarButton(label: "Add Gateway") {
                        model.gatewaySettingsStatus = nil
                        model.lastError = nil
                        showsGatewaySetup = true
                    }
                case .commands:
                    GaryxAddToolbarButton(label: "Add Command") {
                        showsCreateCommand = true
                    }
                case .mcp:
                    GaryxAddToolbarButton(label: "Add Server") {
                        showsCreateMcp = true
                    }
                case .channels:
                    GaryxAddToolbarButton(label: "Add Bot") {
                        showsCreateBot = true
                    }
                case .manage, .provider:
                    EmptyView()
                }
            }
        }
        .fullScreenCover(isPresented: $showsGatewaySetup) {
            GaryxGatewaySetupView(isSheet: true, startsEmpty: true)
        }
        .fullScreenCover(isPresented: $showsCreateBot) {
            GaryxBotAccountForm(account: nil)
        }
        .fullScreenCover(isPresented: $showsCreateCommand) {
            GaryxCreateSlashCommandCard()
        }
        .fullScreenCover(isPresented: $showsCreateMcp) {
            GaryxCreateMcpServerCard()
        }
    }

    private var settingsTitle: String {
        model.activeSettingsTab == .manage ? "Settings" : model.activeSettingsTab.label
    }

    private var settingsLeadingActionLabel: String? {
        model.activeSettingsTab == .manage ? nil : "All Settings"
    }

    private var settingsLeadingAction: (() -> Void)? {
        guard model.activeSettingsTab != .manage else { return nil }
        return {
            model.showSettingsOverview()
        }
    }
}

struct GaryxSettingsTabContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        switch model.activeSettingsTab {
        case .manage:
            GaryxSettingsOverviewContent()
        case .gateway:
            GaryxSettingsDetailContent {
                GaryxSettingsGatewayContent()
            }
        case .provider:
            GaryxSettingsDetailContent {
                GaryxSettingsProviderContent()
            }
        case .channels:
            GaryxSettingsDetailContent {
                GaryxBotsContent()
            }
        case .commands:
            GaryxSettingsDetailContent {
                GaryxCommandsContent()
            }
        case .mcp:
            GaryxSettingsDetailContent {
                GaryxMcpServersContent()
            }
        }
    }
}

struct GaryxSettingsOverviewContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    private var managementPanels: [GaryxMobilePanel] {
        [
            model.dreamsAutoScanEnabled ? .dreams : nil,
            .tasks,
            .skills,
        ].compactMap { $0 }
    }
    private let settingsTabs: [GaryxMobileSettingsTab] = [
        .gateway,
        .provider,
        .channels,
        .commands,
        .mcp,
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            GaryxSettingsOverviewSection(title: "Manage") {
                ForEach(Array(managementPanels.enumerated()), id: \.element.id) { index, panel in
                    GaryxSettingsPanelLinkRow(panel: panel)
                    if index < managementPanels.count - 1 {
                        Divider()
                            .padding(.leading, 54)
                    }
                }
            }

            GaryxSettingsOverviewSection(title: "Settings") {
                GaryxDreamsAutoScanRow()
                Divider()
                    .padding(.leading, 54)

                ForEach(Array(settingsTabs.enumerated()), id: \.element.id) { index, tab in
                    GaryxSettingsTabLinkRow(tab: tab)
                    if index < settingsTabs.count - 1 {
                        Divider()
                            .padding(.leading, 54)
                    }
                }
            }
        }
    }
}

struct GaryxSettingsOverviewSection<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(GaryxFont.caption(weight: .medium))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 16)

            VStack(spacing: 0) {
                content
            }
            .background(GaryxTheme.surface)
        }
    }
}

struct GaryxSettingsDetailContent<Content: View>: View {
    let content: Content

    init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            content
        }
    }
}

struct GaryxSettingsPanelLinkRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let panel: GaryxMobilePanel

    var body: some View {
        GaryxDisclosureListRow(
            title: panel.label,
            subtitle: subtitle,
            systemImage: panel.iconName
        ) {
            model.openPanel(panel)
        }
    }

    private var subtitle: String {
        switch panel {
        case .workspaces:
            "\(model.userWorkspacePaths.count) workspaces"
        case .dreams:
            "\(model.dreams.count) topics"
        case .tasks:
            "\(model.activeTaskCount) active / \(model.tasks.count) total"
        case .workspaceBots:
            "\(model.mobileBotGroups.count) bots / \(visibleWorkspaceCount) workspaces"
        case .agents:
            "\(model.agents.count) agents / \(model.teams.count) teams"
        case .skills:
            "\(model.skills.filter(\.enabled).count) enabled / \(model.skills.count) total"
        default:
            ""
        }
    }

    private var visibleWorkspaceCount: Int {
        model.userWorkspacePaths.count
    }
}

struct GaryxSettingsTabLinkRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let tab: GaryxMobileSettingsTab

    var body: some View {
        GaryxDisclosureListRow(
            title: tab.label,
            subtitle: subtitle,
            systemImage: tab.iconName
        ) {
            model.activeSettingsTab = tab
        }
    }

    private var subtitle: String {
        switch tab {
        case .manage:
            "All mobile settings"
        case .gateway:
            model.gatewayURL.isEmpty ? "Connection and saved gateways" : model.gatewayURL
        case .provider:
            model.providerModelsByType.isEmpty ? "Model providers" : "\(model.providerModelsByType.count) provider types"
        case .channels:
            "\(model.configuredBots.count) configured bots"
        case .commands:
            "\(model.slashCommands.count) slash commands"
        case .mcp:
            "\(model.mcpServers.count) servers"
        }
    }
}

struct GaryxSettingsGatewayContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        // Just the gateway list: the row checkmark already carries the
        // selected state, so no Current block, reconnect button, or status
        // line.
        VStack(alignment: .leading, spacing: 12) {
            if !model.gatewayProfiles.isEmpty {
                GaryxCompactListGroup {
                    ForEach(Array(model.gatewayProfiles.enumerated()), id: \.element.id) { index, profile in
                        GaryxSavedGatewayProfileRow(
                            profile: profile,
                            isCurrent: model.currentGatewayProfile?.id == profile.id
                        )
                        if index < model.gatewayProfiles.count - 1 {
                            GaryxCompactRowDivider()
                        }
                    }
                }
            } else {
                GaryxGatewayEmptyProfilesRow()
            }
        }
    }
}

struct GaryxGatewayEmptyProfilesRow: View {
    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "network")
                .font(GaryxFont.system(size: 14, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 22, height: 22)
            Text("No saved gateways")
                .font(GaryxFont.subheadline(weight: .medium))
                .foregroundStyle(.secondary)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 9)
    }
}

struct GaryxGatewayProfileMenuButton: View {
    @EnvironmentObject private var model: GaryxMobileModel
    var onSelect: ((GaryxGatewayProfile) -> Void)?

    var body: some View {
        if model.gatewayProfiles.isEmpty {
            EmptyView()
        } else {
            Menu {
                ForEach(model.gatewayProfiles) { profile in
                    Button {
                        if let onSelect {
                            onSelect(profile)
                        } else {
                            Task { await model.activateGatewayProfile(profile) }
                        }
                    } label: {
                        Label(profile.gatewayUrl, systemImage: profile.hasToken ? "key.fill" : "network")
                    }
                }
            } label: {
                GaryxToolbarIcon(systemName: "clock.arrow.circlepath")
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Choose gateway")
        }
    }
}

struct GaryxSavedGatewayProfileRow: View {
    @Environment(\.isEnabled) private var isEnabled
    @EnvironmentObject private var model: GaryxMobileModel
    let profile: GaryxGatewayProfile
    let isCurrent: Bool
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false
    @State private var label = ""
    @State private var gatewayUrl = ""
    @State private var token = ""

    var body: some View {
        GaryxRowActionMenu(actions: profileSwipeActions) {
            HStack(spacing: 9) {
                if isCurrent {
                    GaryxSelectionCheckmark(style: .circle, size: 14)
                        .frame(width: 20, height: 20)
                } else {
                    Image(systemName: "network")
                        .font(GaryxFont.system(size: 14, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 20, height: 20)
                }

                VStack(alignment: .leading, spacing: 2) {
                    Text(profile.label)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(profile.gatewayUrl)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                Spacer(minLength: 0)

                if profile.hasToken {
                    Image(systemName: "key.fill")
                        .font(GaryxFont.system(size: 11, weight: .semibold))
                        .foregroundStyle(.secondary)
                }
            }
            .padding(.horizontal, 9)
            .padding(.vertical, 7)
            .contentShape(Rectangle())
            // The settings tab manages saved gateways only; switching lives
            // in the sidebar gateway switcher, so row tap opens the editor.
            .onTapGesture {
                guard isEnabled else { return }
                fillDraft()
                showsEditForm = true
            }
        }
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(
                title: "Edit Gateway",
                canSave: canSaveGateway,
                onSave: saveGateway
            ) {
                VStack(alignment: .leading, spacing: 22) {
                    GaryxFormGroupedSection(title: "Gateway") {
                        GaryxFormTextFieldRow(title: "Name", text: $label)
                        Divider().padding(.leading, 16)
                        GaryxFormTextFieldRow(
                            title: "Gateway URL",
                            text: $gatewayUrl,
                            keyboardType: .URL,
                            textContentType: .URL,
                            autocapitalization: .never,
                            autocorrectionDisabled: true,
                            wrapsValue: true
                        )
                        Divider().padding(.leading, 16)
                        GaryxFormSecureFieldRow(
                            title: "Gateway Token",
                            text: $token,
                            autocapitalization: .never,
                            autocorrectionDisabled: true
                        )
                    }
                }
            }
        }
        .confirmationDialog("Delete gateway?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                model.removeGatewayProfile(profile)
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the saved gateway profile from this device.")
        }
    }

    private var profileSwipeActions: [GaryxRowAction] {
        [
            GaryxRowAction(title: "Edit", systemImage: "pencil") {
                fillDraft()
                showsEditForm = true
            },
            GaryxRowAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                showsDeleteConfirmation = true
            }
        ]
    }

    private func fillDraft() {
        label = profile.label
        gatewayUrl = profile.gatewayUrl
        token = model.gatewayProfileToken(profile)
    }

    private var canSaveGateway: Bool {
        !gatewayUrl.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func saveGateway() {
        guard canSaveGateway else { return }
        if model.updateGatewayProfile(
            profile,
            label: label,
            gatewayUrl: gatewayUrl,
            token: token
        ) {
            showsEditForm = false
        }
    }
}

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
                                )
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

            if let usageDisplay {
                providerUsageBlock(usageDisplay)
                    .padding(.leading, 29)
            }
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
        GaryxProviderUsageDisplayModel.make(from: usage)
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

    @ViewBuilder
    private func providerUsageBlock(_ usageDisplay: GaryxProviderUsageDisplayModel) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Text("Usage")
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
                Text(usageDisplay.summaryText)
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(usageDisplay.available ? .primary : .secondary)
                Text(usageDisplay.detailText)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.75)
            }
            if !usageDisplay.models.isEmpty {
                VStack(alignment: .leading, spacing: 3) {
                    ForEach(usageDisplay.models) { model in
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            Text(model.title)
                                .font(GaryxFont.caption())
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .minimumScaleFactor(0.72)
                            Spacer(minLength: 6)
                            Text(model.remainingText)
                                .font(GaryxFont.caption(weight: .semibold))
                                .foregroundStyle(.primary)
                                .lineLimit(1)
                            Text(model.detailText)
                                .font(GaryxFont.caption())
                                .foregroundStyle(.tertiary)
                                .lineLimit(1)
                                .minimumScaleFactor(0.7)
                        }
                    }
                }
            }
        }
    }
}

struct GaryxModelProviderDefaultsSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let provider: GaryxModelProviderDefault
    @State private var modelName = ""
    @State private var reasoningEffort = ""
    @State private var isSaving = false

    var body: some View {
        GaryxFormSheet(
            title: "\(providerPresentation.displayName) Defaults",
            canSave: !isSaving,
            onSave: saveDefaults
        ) {
            VStack(alignment: .leading, spacing: 22) {
                GaryxFormGroupedSection(title: "Provider") {
                    GaryxFormReadOnlyRow(title: "Name", value: providerPresentation.displayName)
                    Divider().padding(.leading, 16)
                    GaryxFormReadOnlyRow(title: "Type", value: provider.providerType)
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
                }
            }
        }
        .onAppear(perform: fillDraft)
        .task {
            await model.loadProviderModels(providerType: provider.providerType)
        }
        .onChange(of: modelName) { _, _ in
            reasoningEffort = GaryxThreadModelOverridePresentation.sanitizedReasoningEffort(
                providerModels: catalog,
                model: modelName,
                reasoningEffort: reasoningEffort
            ) ?? ""
        }
    }

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

    private var defaultModelLabel: String {
        let defaultModel = catalog?.defaultModel?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !defaultModel.isEmpty { return "Provider default: \(defaultModel)" }
        let fallback = provider.fallbackDefaultModel.trimmingCharacters(in: .whitespacesAndNewlines)
        if !fallback.isEmpty { return "Provider default: \(fallback)" }
        return "Provider default"
    }

    private func fillDraft() {
        modelName = GaryxModelProviderDefaults.configuredDefaultModel(
            in: model.gatewaySettingsDocument,
            provider: provider
        )
        reasoningEffort = GaryxModelProviderDefaults.configuredReasoningEffort(
            in: model.gatewaySettingsDocument,
            provider: provider
        )
    }

    private func saveDefaults() {
        guard !isSaving else { return }
        isSaving = true
        Task {
            let didSave = await model.updateModelProviderDefaults(
                provider: provider,
                modelName: modelName,
                reasoningEffort: reasoningEffort
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

private struct GaryxProviderDefaultPickerRow: View {
    let title: String
    @Binding var value: String
    let placeholder: String
    let options: [GaryxProviderModelOption]
    let iconName: String

    var body: some View {
        GaryxFormRow(title: title) {
            Menu {
                Button("Provider default") {
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
            } label: {
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
