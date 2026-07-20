import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct GaryxMobileSettingsPanel: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let tab: GaryxMobileSettingsTab
    @State private var showsGatewaySetup = false
    @State private var showsCreateBot = false
    @State private var showsCreateCommand = false
    @State private var showsCreateMcp = false

    var body: some View {
        GaryxPanelScaffold(
            title: settingsTitle,
            subtitle: tab.label,
            onRefresh: { await model.connectAndRefresh() },
            background: GaryxTheme.background
        ) {
            VStack(alignment: .leading, spacing: 12) {
                GaryxSettingsTabContent(tab: tab)
            }
        } actions: {
            HStack(spacing: 8) {
                switch tab {
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
        .garyxFullScreenCover(isPresented: $showsGatewaySetup) {
            GaryxGatewaySetupView(isSheet: true, startsEmpty: true)
        }
        .garyxFullScreenCover(isPresented: $showsCreateBot) {
            GaryxBotAccountForm(account: nil)
        }
        .garyxSheet(isPresented: $showsCreateCommand) {
            GaryxCreateSlashCommandCard()
        }
        .garyxFullScreenCover(isPresented: $showsCreateMcp) {
            GaryxCreateMcpServerCard()
        }
    }

    private var settingsTitle: String {
        tab == .manage ? "Settings" : tab.label
    }
}

struct GaryxSettingsTabContent: View {
    let tab: GaryxMobileSettingsTab

    var body: some View {
        switch tab {
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
        [.skills]
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
        VStack(alignment: .leading, spacing: 7) {
            Text(title)
                .font(Font.footnote.weight(.semibold))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 4)

            VStack(spacing: 0) {
                content
            }
            .background(
                Color(.secondarySystemGroupedBackground),
                in: RoundedRectangle(cornerRadius: 15, style: .continuous)
            )
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
        case .workspaceBots:
            "\(model.mobileBotGroups.count) bots / \(visibleWorkspaceCount) workspaces"
        case .agents:
            "\(model.agents.count) agents"
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
            model.openSettings(tab: tab, source: .current)
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
                .font(GaryxFont.fixedSystem(size: 14, weight: .semibold))
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
    @State private var headers = ""

    var body: some View {
        GaryxRowActionMenu(actions: profileSwipeActions) {
            HStack(spacing: 9) {
                if isCurrent {
                    GaryxSelectionCheckmark(style: .circle, size: 14)
                        .frame(width: 20, height: 20)
                } else {
                    Image(systemName: "network")
                        .font(GaryxFont.fixedSystem(size: 14, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 20, height: 20)
                }

                VStack(alignment: .leading, spacing: 2) {
                    Text(profile.label)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .garyxReadingLineLimit()
                    Text(profile.gatewayUrl)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .garyxReadingLineLimit()
                }

                Spacer(minLength: 0)

                if profile.hasToken {
                    Image(systemName: "key.fill")
                        .font(GaryxFont.fixedSystem(size: 11, weight: .semibold))
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
        .garyxSheet(isPresented: $showsEditForm) {
            GaryxFormSheet(
                title: "Edit Gateway",
                canSave: canSaveGateway,
                onSave: saveGateway
            ) {
                GaryxFormGroupedSection(title: "Gateway") {
                    GaryxFormTextFieldRow(title: "Name", text: $label)
                    GaryxFormTextFieldRow(
                        title: "Gateway URL",
                        text: $gatewayUrl,
                        valuePlacement: .below,
                        keyboardType: .URL,
                        textContentType: .URL,
                        autocapitalization: .never,
                        autocorrectionDisabled: true,
                        wrapsValue: true
                    )
                    GaryxFormSecureFieldRow(
                        title: "Gateway Token",
                        text: $token,
                        valuePlacement: .below,
                        autocapitalization: .never,
                        autocorrectionDisabled: true
                    )
                    GaryxGatewayHeadersEditor(text: $headers)
                }
            }
            .presentationDetents([.large])
            .presentationDragIndicator(.visible)
        }
        .garyxConfirmationDialog("Delete gateway?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
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
        headers = profile.gatewayHeaders
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
            token: token,
            headers: headers
        ) {
            showsEditForm = false
        }
    }
}
