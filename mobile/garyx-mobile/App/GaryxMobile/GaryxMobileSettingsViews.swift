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
            .autoResearch,
            .agents,
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
        case .autoResearch:
            "\(model.runningResearchCount) active / \(model.autoResearchRuns.count) total"
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
        VStack(alignment: .leading, spacing: 12) {
            GaryxSectionBlock(title: "Current") {
                GaryxGatewayCurrentRow()
            }

            if !model.gatewayProfiles.isEmpty {
                GaryxSectionBlock(title: "Gateways") {
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
                }
            } else {
                GaryxSectionBlock(title: "Gateways") {
                    GaryxGatewayEmptyProfilesRow()
                }
            }

            if let status = model.gatewaySettingsStatus, !status.isEmpty {
                Text(status)
                    .font(GaryxFont.caption(weight: .medium))
                    .foregroundStyle(GaryxTheme.accent)
                    .padding(.horizontal, 2)
            }
        }
    }
}

struct GaryxGatewayCurrentRow: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: currentIcon)
                .font(GaryxFont.system(size: 15, weight: .semibold))
                .foregroundStyle(currentColor)
                .frame(width: 22, height: 22)

            VStack(alignment: .leading, spacing: 2) {
                Text(currentTitle)
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                Text(model.gatewayURL.isEmpty ? "No gateway selected" : model.gatewayURL)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer(minLength: 0)

            Button {
                Task { await model.connectAndRefresh() }
            } label: {
                Image(systemName: "arrow.clockwise")
                    .font(GaryxFont.system(size: 13, weight: .semibold))
                    .frame(width: 34, height: 34)
                    .background(Color(.secondarySystemFill), in: Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Reconnect gateway")
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 8)
    }

    private var currentTitle: String {
        switch model.connectionState {
        case .ready(let version):
            if let version, !version.isEmpty {
                return "Connected \(version)"
            }
            return "Connected"
        case .checking:
            return "Connecting"
        case .failed:
            return "Connection failed"
        case .disconnected:
            return "Not connected"
        }
    }

    private var currentIcon: String {
        switch model.connectionState {
        case .ready:
            return "checkmark.circle.fill"
        case .checking:
            return "arrow.triangle.2.circlepath"
        case .failed:
            return "exclamationmark.circle.fill"
        case .disconnected:
            return "network"
        }
    }

    private var currentColor: Color {
        switch model.connectionState {
        case .ready:
            return GaryxTheme.accent
        case .checking:
            return .secondary
        case .failed:
            return GaryxTheme.danger
        case .disconnected:
            return .secondary
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
            .onTapGesture {
                Task { await model.activateGatewayProfile(profile) }
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
                        TextField("Name", text: $label)
                            .garyxFormTextField()
                        Divider().padding(.leading, 16)
                        TextField("Gateway URL", text: $gatewayUrl)
                            .keyboardType(.URL)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .garyxFormTextField()
                        Divider().padding(.leading, 16)
                        SecureField("Gateway Token", text: $token)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .garyxFormTextField()
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
            GaryxRowAction(title: "Switch", systemImage: "arrow.triangle.2.circlepath", tone: .accent) {
                Task { await model.activateGatewayProfile(profile) }
            },
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

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if !model.providerModelsByType.isEmpty {
                GaryxSectionBlock(title: "Model Providers") {
                    GaryxCompactListGroup {
                        let providers = model.providerModelsByType
                            .values
                            .sorted { lhs, rhs in
                                let lhsName = GaryxProviderPresentation.displayName(for: lhs.providerType)
                                let rhsName = GaryxProviderPresentation.displayName(for: rhs.providerType)
                                if lhsName != rhsName {
                                    return lhsName < rhsName
                                }
                                return lhs.providerType < rhs.providerType
                            }
                        ForEach(Array(providers.enumerated()), id: \.element.providerType) { index, provider in
                            GaryxProviderModelsRow(provider: provider)
                            if index < providers.count - 1 {
                                GaryxCompactRowDivider()
                            }
                        }
                    }
                }
            }
        }
    }
}

struct GaryxProviderModelsRow: View {
    let provider: GaryxProviderModels

    var body: some View {
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

            GaryxStatusPill(text: hasError ? "Error" : "Ready", tone: hasError ? .danger : .good)
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 7)
    }

    private var iconName: String {
        providerPresentation.symbolName ?? "cpu"
    }

    private var providerPresentation: GaryxProviderPresentation {
        GaryxProviderPresentation.make(providerType: provider.providerType)
    }

    private var hasError: Bool {
        let error = provider.error?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return !error.isEmpty
    }

    private var detail: String {
        var parts: [String] = []
        if let defaultModel = provider.defaultModel?.trimmingCharacters(in: .whitespacesAndNewlines), !defaultModel.isEmpty {
            parts.append("Default \(defaultModel)")
        }
        if provider.supportsModelSelection {
            parts.append("\(provider.models.count) models")
        }
        if provider.supportsReasoningEffortSelection {
            parts.append("\(provider.reasoningEfforts.count) reasoning")
        }
        if provider.supportsServiceTierSelection {
            parts.append("\(provider.serviceTiers.count) tiers")
        }
        if parts.isEmpty {
            if hasError {
                return "Model metadata unavailable"
            }
            return provider.source.isEmpty ? "Provider metadata" : provider.source.capitalized
        }
        return parts.joined(separator: " · ")
    }
}
