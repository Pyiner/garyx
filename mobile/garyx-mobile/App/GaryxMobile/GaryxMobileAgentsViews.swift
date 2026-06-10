import Foundation
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

private enum GaryxAgentCreationSheet: String, Identifiable {
    case agent
    case team

    var id: String { rawValue }

    var title: String {
        switch self {
        case .agent:
            "New Agent"
        case .team:
            "New Team"
        }
    }
}

private enum GaryxAgentsTab: String, CaseIterable, Identifiable {
    case agents = "Agents"
    case teams = "Teams"

    var id: String { rawValue }
}

struct GaryxAgentsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var creationSheet: GaryxAgentCreationSheet?
    @State private var selectedTab: GaryxAgentsTab = .agents

    var body: some View {
        GaryxPanelScaffold(
            title: "Agents",
            subtitle: "\(model.agents.count) agents / \(model.teams.count) teams",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            VStack(alignment: .leading, spacing: 18) {
                Picker("Agent type", selection: $selectedTab) {
                    ForEach(GaryxAgentsTab.allCases) { tab in
                        Text(tab.rawValue).tag(tab)
                    }
                }
                .pickerStyle(.segmented)

                switch selectedTab {
                case .agents:
                    GaryxSectionBlock(title: "Agents") {
                        GaryxCompactListGroup {
                            ForEach(Array(model.agents.enumerated()), id: \.element.id) { index, agent in
                                GaryxAgentCard(agent: agent)
                                if index < model.agents.count - 1 {
                                    GaryxCompactRowDivider()
                                }
                            }
                        }
                    }
                case .teams:
                    GaryxSectionBlock(title: "Teams") {
                        GaryxCompactListGroup {
                            ForEach(Array(model.teams.enumerated()), id: \.element.id) { index, team in
                                GaryxTeamCard(team: team)
                                if index < model.teams.count - 1 {
                                    GaryxCompactRowDivider()
                                }
                            }
                        }
                    }
                }
            }
        } actions: {
            GaryxAddToolbarButton(label: selectedTab == .agents ? "New Agent" : "New Team") {
                creationSheet = selectedTab == .agents ? .agent : .team
            }
        }
        .fullScreenCover(item: $creationSheet) { sheet in
            switch sheet {
            case .agent:
                GaryxCreateAgentCard()
            case .team:
                GaryxCreateTeamCard()
            }
        }
        .fullScreenCover(item: $model.selectedAgentDetail) { agent in
            GaryxFormSheet(title: "Agent Detail") {
                GaryxAgentDetailCard(agent: agent)
            }
        }
        .fullScreenCover(item: $model.selectedTeamDetail) { team in
            GaryxFormSheet(title: "Team Detail") {
                GaryxTeamDetailCard(team: team)
            }
        }
    }
}

struct GaryxAgentDetailCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let agent: GaryxAgentSummary
    @State private var showsEditForm = false

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            GaryxAgentFormContent(
                mode: .readOnly,
                agentId: .constant(displayAgent.id),
                displayName: .constant(displayAgent.displayName),
                providerType: .constant(displayAgent.providerType),
                modelName: .constant(displayAgent.model),
                modelReasoningEffort: .constant(displayAgent.modelReasoningEffort),
                workspace: .constant(displayAgent.defaultWorkspaceDir),
                avatarDataUrl: .constant(displayAgent.avatarDataUrl),
                systemPrompt: .constant(displayAgent.systemPrompt),
                builtIn: displayAgent.builtIn,
                workspacePaths: model.userWorkspacePaths
            )

            if !displayAgent.builtIn {
                Button {
                    showsEditForm = true
                } label: {
                    Label("Edit Agent", systemImage: "pencil")
                        .font(GaryxFont.callout(weight: .semibold))
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(GaryxPrimaryWideButtonStyle())
            }
        }
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxAgentEditSheet(agent: displayAgent) { updatedAgent in
                model.selectedAgentDetail = updatedAgent
            }
        }
    }

    private var displayAgent: GaryxAgentSummary {
        model.agents.first(where: { $0.id == agent.id }) ?? agent
    }
}

struct GaryxTeamDetailCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let team: GaryxTeamSummary
    @State private var showsEditForm = false

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            GaryxTeamFormContent(
                mode: .readOnly,
                teamId: .constant(displayTeam.id),
                displayName: .constant(displayTeam.displayName),
                avatarDataUrl: .constant(displayTeam.avatarDataUrl),
                leaderAgentId: .constant(displayTeam.leaderAgentId),
                memberAgentIds: .constant(displayTeam.memberAgentIds.joined(separator: ", ")),
                workflowText: .constant(displayTeam.workflowText),
                agents: model.agents
            )

            Button {
                showsEditForm = true
            } label: {
                Label("Edit Team", systemImage: "pencil")
                    .font(GaryxFont.callout(weight: .semibold))
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(GaryxPrimaryWideButtonStyle())
        }
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxTeamEditSheet(team: displayTeam) { updatedTeam in
                model.selectedTeamDetail = updatedTeam
            }
        }
    }

    private var displayTeam: GaryxTeamSummary {
        model.teams.first(where: { $0.id == team.id }) ?? team
    }
}

private enum GaryxAgentFormMode {
    case editable
    case readOnly

    var isEditable: Bool {
        self == .editable
    }
}

private struct GaryxAgentReadOnlyTextRow: View {
    let title: String
    let value: String
    var placeholder = "None"

    var body: some View {
        GaryxFormReadOnlyMultilineRow(
            title: title,
            value: value,
            placeholder: placeholder,
            minHeight: 34,
            valuePlacement: .below
        )
    }
}

private struct GaryxAgentAvatarPreviewSection: View {
    let kind: GaryxAgentAvatarKind
    let identifier: String
    let displayName: String
    let providerType: String
    let avatarDataUrl: String
    var builtIn = false

    var body: some View {
        GaryxFormGroupedSection(title: "Avatar") {
            GaryxAgentAvatarView(
                agentId: trimmedIdentifier,
                avatarDataUrl: avatarDataUrl,
                kind: kind == .team ? .team : .agent,
                label: avatarLabel,
                providerType: providerType,
                builtIn: builtIn,
                diameter: 96
            )
            .accessibilityLabel("\(kind == .team ? "Team" : "Agent") avatar preview")
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .center)
        }
    }

    private var trimmedIdentifier: String {
        identifier.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var avatarLabel: String {
        let name = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        if !name.isEmpty {
            return name
        }
        return trimmedIdentifier.isEmpty ? (kind == .team ? "Team" : "Agent") : trimmedIdentifier
    }
}

private struct GaryxAgentFormContent: View {
    let mode: GaryxAgentFormMode
    @Binding var agentId: String
    @Binding var displayName: String
    @Binding var providerType: String
    @Binding var modelName: String
    @Binding var modelReasoningEffort: String
    @Binding var workspace: String
    @Binding var avatarDataUrl: String
    @Binding var systemPrompt: String
    var builtIn = false
    let workspacePaths: [String]
    var onGenerate: ((String) async -> String?)?
    var onError: ((String) -> Void)?

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            if mode.isEditable, let onGenerate, let onError {
                GaryxAvatarEditorSection(
                    kind: .agent,
                    identifier: agentId,
                    displayName: displayName,
                    providerType: providerType,
                    builtIn: builtIn,
                    avatarDataUrl: $avatarDataUrl,
                    onGenerate: onGenerate,
                    onError: onError
                )
            } else {
                GaryxAgentAvatarPreviewSection(
                    kind: .agent,
                    identifier: agentId,
                    displayName: displayName,
                    providerType: providerType,
                    avatarDataUrl: avatarDataUrl,
                    builtIn: builtIn
                )
            }

            GaryxFormGroupedSection(title: "Identity") {
                if mode.isEditable {
                    GaryxFormTextFieldRow(
                        title: "Agent ID",
                        text: $agentId,
                        autocapitalization: .never,
                        autocorrectionDisabled: true
                    )
                    Divider().padding(.leading, 16)
                    GaryxFormTextFieldRow(
                        title: "Display name",
                        text: $displayName,
                        placeholder: "Optional"
                    )
                } else {
                    GaryxAgentReadOnlyTextRow(title: "Agent ID", value: agentId)
                    Divider().padding(.leading, 16)
                    GaryxAgentReadOnlyTextRow(title: "Display name", value: displayName)
                    Divider().padding(.leading, 16)
                    GaryxAgentReadOnlyTextRow(title: "Type", value: builtIn ? "Built-in" : "Custom")
                }
            }

            GaryxFormGroupedSection(title: "Model") {
                if mode.isEditable {
                    GaryxAgentProviderSelectionRow(
                        providerType: $providerType,
                        modelName: $modelName,
                        modelReasoningEffort: $modelReasoningEffort
                    )
                    Divider().padding(.leading, 16)
                    GaryxAgentModelSelectionRow(
                        providerType: $providerType,
                        modelName: $modelName
                    )
                    GaryxAgentReasoningEffortSelectionRow(
                        providerType: $providerType,
                        modelName: $modelName,
                        reasoningEffort: $modelReasoningEffort
                    )
                } else {
                    GaryxFormReadOnlyRow(title: "Provider", value: garyxAgentProviderLabel(for: providerType))
                    Divider().padding(.leading, 16)
                    GaryxFormReadOnlyRow(title: "Model", value: modelDisplayValue)
                    if !modelReasoningEffort.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        Divider().padding(.leading, 16)
                        GaryxFormReadOnlyRow(title: "Thinking level", value: modelReasoningEffort)
                    }
                }
            }

            GaryxFormGroupedSection(title: "Defaults") {
                if mode.isEditable {
                    GaryxWorkspacePathSelectionRow(
                        title: "Default workspace",
                        path: $workspace,
                        workspacePaths: workspacePaths,
                        placeholder: "Optional",
                        allowsEmpty: true
                    )
                    Divider().padding(.leading, 16)
                    GaryxFormTextAreaRow(
                        title: "System Prompt",
                        text: $systemPrompt,
                        minHeight: 132,
                        lineLimits: 2...6
                    )
                } else {
                    GaryxFormReadOnlyMultilineRow(
                        title: "Default workspace",
                        value: workspace,
                        placeholder: "None",
                        minHeight: 44,
                        valuePlacement: .below
                    )
                    Divider().padding(.leading, 16)
                    GaryxFormReadOnlyMultilineRow(
                        title: "System Prompt",
                        value: systemPrompt,
                        placeholder: "None",
                        minHeight: 132,
                        valuePlacement: .below
                    )
                }
            }
        }
    }

    private var modelDisplayValue: String {
        let trimmed = modelName.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "Provider default" : modelName
    }
}

private struct GaryxTeamFormContent: View {
    let mode: GaryxAgentFormMode
    @Binding var teamId: String
    @Binding var displayName: String
    @Binding var avatarDataUrl: String
    @Binding var leaderAgentId: String
    @Binding var memberAgentIds: String
    @Binding var workflowText: String
    let agents: [GaryxAgentSummary]
    var onGenerate: ((String) async -> String?)?
    var onError: ((String) -> Void)?

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            if mode.isEditable, let onGenerate, let onError {
                GaryxAvatarEditorSection(
                    kind: .team,
                    identifier: teamId,
                    displayName: displayName,
                    providerType: "",
                    avatarDataUrl: $avatarDataUrl,
                    onGenerate: onGenerate,
                    onError: onError
                )
            } else {
                GaryxAgentAvatarPreviewSection(
                    kind: .team,
                    identifier: teamId,
                    displayName: displayName,
                    providerType: "",
                    avatarDataUrl: avatarDataUrl
                )
            }

            GaryxFormGroupedSection(title: "Identity") {
                if mode.isEditable {
                    GaryxFormTextFieldRow(
                        title: "Team ID",
                        text: $teamId,
                        autocapitalization: .never,
                        autocorrectionDisabled: true
                    )
                    Divider().padding(.leading, 16)
                    GaryxFormTextFieldRow(
                        title: "Display name",
                        text: $displayName,
                        placeholder: "Optional"
                    )
                } else {
                    GaryxAgentReadOnlyTextRow(title: "Team ID", value: teamId)
                    Divider().padding(.leading, 16)
                    GaryxAgentReadOnlyTextRow(title: "Display name", value: displayName)
                }
            }

            GaryxFormGroupedSection(title: "Members") {
                if mode.isEditable {
                    GaryxTeamLeaderSelectionRow(
                        leaderAgentId: $leaderAgentId,
                        memberAgentIds: $memberAgentIds,
                        agents: agents
                    )
                    Divider().padding(.leading, 16)
                    GaryxTeamMembersSelectionRow(
                        leaderAgentId: $leaderAgentId,
                        memberAgentIds: $memberAgentIds,
                        agents: agents
                    )
                } else {
                    GaryxFormReadOnlyRow(title: "Leader", value: agentLabel(for: leaderAgentId))
                    Divider().padding(.leading, 16)
                    GaryxFormReadOnlyMultilineRow(
                        title: "Members",
                        value: memberLabels,
                        placeholder: "No members",
                        minHeight: 52,
                        valuePlacement: .below
                    )
                }
            }

            GaryxFormGroupedSection(title: "Workflow") {
                if mode.isEditable {
                    GaryxFormTextAreaRow(
                        title: "Workflow",
                        text: $workflowText,
                        minHeight: 132,
                        lineLimits: 2...6
                    )
                } else {
                    GaryxFormReadOnlyMultilineRow(
                        title: "Workflow",
                        value: workflowText,
                        placeholder: "None",
                        minHeight: 132,
                        valuePlacement: .below
                    )
                }
            }
        }
    }

    private var memberLabels: String {
        let ids = garyxTeamMemberIds(from: memberAgentIds)
        guard !ids.isEmpty else { return "" }
        return ids.map { agentLabel(for: $0) }.joined(separator: "\n")
    }

    private func agentLabel(for agentId: String) -> String {
        let trimmed = agentId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "" }
        guard let agent = agents.first(where: { $0.id == trimmed }) else { return trimmed }
        return agent.displayName.isEmpty ? agent.id : "\(agent.displayName) (\(agent.id))"
    }
}

struct GaryxCreateAgentCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var agentId = ""
    @State private var displayName = ""
    @State private var providerType = "codex_app_server"
    @State private var modelName = ""
    @State private var modelReasoningEffort = ""
    @State private var workspace = ""
    @State private var avatarDataUrl = ""
    @State private var systemPrompt = ""

    var body: some View {
        GaryxFormSheet(
            title: "New Agent",
            canSave: canCreate,
            onSave: { Task { await createAgent() } }
        ) {
            GaryxAgentFormContent(
                mode: .editable,
                agentId: $agentId,
                displayName: $displayName,
                providerType: $providerType,
                modelName: $modelName,
                modelReasoningEffort: $modelReasoningEffort,
                workspace: $workspace,
                avatarDataUrl: $avatarDataUrl,
                systemPrompt: $systemPrompt,
                workspacePaths: model.userWorkspacePaths
            ) { stylePrompt in
                await model.generateAvatar(
                    kind: .agent,
                    identifier: agentId,
                    displayName: displayName,
                    stylePrompt: stylePrompt
                )
            } onError: { message in
                model.lastError = message
            }
        }
    }

    private var canCreate: Bool {
        !agentId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !providerType.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func createAgent() async {
        guard canCreate else { return }
        if await model.createAgent(
            agentId: agentId,
            displayName: displayName,
            providerType: providerType,
            modelName: modelName,
            modelReasoningEffort: modelReasoningEffort,
            workspace: workspace,
            avatarDataUrl: avatarDataUrl,
            systemPrompt: systemPrompt
        ) {
            dismiss()
        }
    }
}

struct GaryxCreateTeamCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var teamId = ""
    @State private var displayName = ""
    @State private var avatarDataUrl = ""
    @State private var leaderAgentId = ""
    @State private var memberAgentIds = ""
    @State private var workflowText = ""

    var body: some View {
        GaryxFormSheet(
            title: "New Team",
            canSave: canCreate,
            onSave: { Task { await createTeam() } }
        ) {
            GaryxTeamFormContent(
                mode: .editable,
                teamId: $teamId,
                displayName: $displayName,
                avatarDataUrl: $avatarDataUrl,
                leaderAgentId: $leaderAgentId,
                memberAgentIds: $memberAgentIds,
                workflowText: $workflowText,
                agents: model.agents
            ) { stylePrompt in
                await model.generateAvatar(
                    kind: .team,
                    identifier: teamId,
                    displayName: displayName,
                    stylePrompt: stylePrompt
                )
            } onError: { message in
                model.lastError = message
            }
        }
    }

    private var canCreate: Bool {
        !teamId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !leaderAgentId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func createTeam() async {
        guard canCreate else { return }
        if await model.createTeam(
            teamId: teamId,
            displayName: displayName,
            leaderAgentId: leaderAgentId,
            memberAgentIds: memberAgentIds,
            workflowText: workflowText,
            avatarDataUrl: avatarDataUrl
        ) {
            dismiss()
        }
    }
}

private struct GaryxAgentEditSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let agent: GaryxAgentSummary
    var onSaved: ((GaryxAgentSummary) -> Void)?
    @State private var agentId = ""
    @State private var displayName = ""
    @State private var providerType = ""
    @State private var modelName = ""
    @State private var modelReasoningEffort = ""
    @State private var workspace = ""
    @State private var avatarDataUrl = ""
    @State private var systemPrompt = ""

    var body: some View {
        GaryxFormSheet(
            title: "Edit Agent",
            canSave: canSaveAgent,
            onSave: { Task { await saveAgent() } }
        ) {
            GaryxAgentFormContent(
                mode: .editable,
                agentId: $agentId,
                displayName: $displayName,
                providerType: $providerType,
                modelName: $modelName,
                modelReasoningEffort: $modelReasoningEffort,
                workspace: $workspace,
                avatarDataUrl: $avatarDataUrl,
                systemPrompt: $systemPrompt,
                builtIn: agent.builtIn,
                workspacePaths: model.userWorkspacePaths
            ) { stylePrompt in
                await model.generateAvatar(
                    kind: .agent,
                    identifier: agentId,
                    displayName: displayName,
                    stylePrompt: stylePrompt
                )
            } onError: { message in
                model.lastError = message
            }
        }
        .onAppear(perform: fillDraft)
    }

    private var canSaveAgent: Bool {
        !agentId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !providerType.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func fillDraft() {
        agentId = agent.id
        displayName = agent.displayName
        providerType = agent.providerType
        modelName = agent.model
        modelReasoningEffort = agent.modelReasoningEffort
        workspace = agent.defaultWorkspaceDir
        avatarDataUrl = agent.avatarDataUrl
        systemPrompt = agent.systemPrompt
    }

    private func saveAgent() async {
        guard canSaveAgent else { return }
        guard let updated = await model.updateAgent(
            agent,
            agentId: agentId,
            displayName: displayName,
            providerType: providerType,
            modelName: modelName,
            modelReasoningEffort: modelReasoningEffort,
            workspace: workspace,
            avatarDataUrl: avatarDataUrl,
            systemPrompt: systemPrompt
        ) else { return }
        dismiss()
        onSaved?(updated)
    }
}

private struct GaryxTeamEditSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let team: GaryxTeamSummary
    var onSaved: ((GaryxTeamSummary) -> Void)?
    @State private var teamId = ""
    @State private var displayName = ""
    @State private var avatarDataUrl = ""
    @State private var leaderAgentId = ""
    @State private var memberAgentIds = ""
    @State private var workflowText = ""

    var body: some View {
        GaryxFormSheet(
            title: "Edit Team",
            canSave: canSaveTeam,
            onSave: { Task { await saveTeam() } }
        ) {
            GaryxTeamFormContent(
                mode: .editable,
                teamId: $teamId,
                displayName: $displayName,
                avatarDataUrl: $avatarDataUrl,
                leaderAgentId: $leaderAgentId,
                memberAgentIds: $memberAgentIds,
                workflowText: $workflowText,
                agents: model.agents
            ) { stylePrompt in
                await model.generateAvatar(
                    kind: .team,
                    identifier: teamId,
                    displayName: displayName,
                    stylePrompt: stylePrompt
                )
            } onError: { message in
                model.lastError = message
            }
        }
        .onAppear(perform: fillDraft)
    }

    private var canSaveTeam: Bool {
        !teamId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !leaderAgentId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func fillDraft() {
        teamId = team.id
        displayName = team.displayName
        avatarDataUrl = team.avatarDataUrl
        leaderAgentId = team.leaderAgentId
        memberAgentIds = team.memberAgentIds.joined(separator: ", ")
        workflowText = team.workflowText
    }

    private func saveTeam() async {
        guard canSaveTeam else { return }
        guard let updated = await model.updateTeam(
            team,
            teamId: teamId,
            displayName: displayName,
            leaderAgentId: leaderAgentId,
            memberAgentIds: memberAgentIds,
            workflowText: workflowText,
            avatarDataUrl: avatarDataUrl
        ) else { return }
        dismiss()
        onSaved?(updated)
    }
}

private struct GaryxAvatarEditorSection: View {
    let kind: GaryxAgentAvatarKind
    let identifier: String
    let displayName: String
    let providerType: String
    var builtIn = false
    @Binding var avatarDataUrl: String
    let onGenerate: (String) async -> String?
    let onError: (String) -> Void

    @State private var editorState = GaryxMobileAvatarEditorState()
    @State private var selectedPhotoItem: PhotosPickerItem?
    @State private var showsStyleSheet = false
    @State private var workTask: Task<Void, Never>?

    var body: some View {
        GaryxFormGroupedSection(title: "Avatar") {
            VStack(alignment: .center, spacing: 16) {
                GaryxAgentAvatarView(
                    agentId: trimmedIdentifier,
                    avatarDataUrl: avatarDataUrl,
                    kind: targetKind,
                    label: avatarLabel,
                    providerType: providerType,
                    builtIn: builtIn,
                    diameter: 96
                )
                .accessibilityLabel("\(kind == .team ? "Team" : "Agent") avatar preview")

                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 10) {
                        avatarActions
                    }
                    VStack(spacing: 10) {
                        avatarActions
                    }
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .center)
        }
        .sheet(isPresented: $showsStyleSheet) {
            GaryxAvatarStyleSheet(
                isGenerating: editorState.isGenerating,
                canGenerate: canGenerate
            ) { stylePrompt in
                startGeneration(stylePrompt: stylePrompt)
            }
        }
        .onChange(of: selectedPhotoItem) { _, item in
            guard let item else { return }
            startUpload(item)
        }
        .onDisappear {
            cancelWork()
        }
    }

    @ViewBuilder
    private var avatarActions: some View {
        PhotosPicker(selection: $selectedPhotoItem, matching: .images) {
            GaryxAvatarEditorActionLabel(
                title: editorState.isUploading ? "Uploading" : "Upload",
                systemName: "photo",
                isLoading: editorState.isUploading
            )
        }
        .disabled(editorState.isBusy)
        .accessibilityLabel("Upload avatar")

        Button {
            showsStyleSheet = true
        } label: {
            GaryxAvatarEditorActionLabel(
                title: editorState.isGenerating ? "Generating" : "Generate",
                systemName: "sparkles",
                isLoading: editorState.isGenerating
            )
        }
        .buttonStyle(.plain)
        .disabled(editorState.isBusy || !canGenerate)
        .accessibilityLabel("Generate avatar")

    }

    private func startGeneration(stylePrompt: String) {
        guard workTask == nil else { return }
        workTask = Task {
            await generateAvatar(stylePrompt: stylePrompt)
        }
    }

    private func startUpload(_ item: PhotosPickerItem) {
        guard workTask == nil else {
            selectedPhotoItem = nil
            return
        }
        workTask = Task {
            await uploadAvatar(from: item)
        }
    }

    private func cancelWork() {
        workTask?.cancel()
        workTask = nil
        selectedPhotoItem = nil
        editorState.reset()
    }

    private var targetKind: GaryxMobileAgentTarget.Kind {
        kind == .team ? .team : .agent
    }

    private var trimmedIdentifier: String {
        identifier.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var avatarLabel: String {
        let name = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        if !name.isEmpty {
            return name
        }
        return trimmedIdentifier.isEmpty ? (kind == .team ? "Team" : "Agent") : trimmedIdentifier
    }

    private var canGenerate: Bool {
        !trimmedIdentifier.isEmpty || !displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var generationFingerprint: String {
        [
            kind.rawValue,
            trimmedIdentifier,
            displayName.trimmingCharacters(in: .whitespacesAndNewlines),
            providerType.trimmingCharacters(in: .whitespacesAndNewlines),
            String(avatarDataUrl.count),
            String(avatarDataUrl.prefix(80)),
            String(avatarDataUrl.suffix(80)),
        ].joined(separator: "\u{1F}")
    }

    private var uploadFingerprint: String {
        [
            kind.rawValue,
            String(avatarDataUrl.count),
            String(avatarDataUrl.prefix(80)),
            String(avatarDataUrl.suffix(80)),
        ].joined(separator: "\u{1F}")
    }

    @MainActor
    private func generateAvatar(stylePrompt: String) async {
        guard canGenerate, !editorState.isBusy else { return }
        let fingerprint = generationFingerprint
        let requestId = editorState.begin(.generate, fingerprint: fingerprint)
        defer { finishWork(requestId: requestId) }

        guard let generated = await onGenerate(stylePrompt) else { return }
        guard canApplyCurrentResult(requestId: requestId, fingerprint: generationFingerprint) else { return }
        avatarDataUrl = generated
    }

    @MainActor
    private func uploadAvatar(from item: PhotosPickerItem) async {
        guard !editorState.isBusy else {
            selectedPhotoItem = nil
            return
        }
        let fingerprint = uploadFingerprint
        let requestId = editorState.begin(.upload, fingerprint: fingerprint)
        defer {
            selectedPhotoItem = nil
            finishWork(requestId: requestId)
        }

        do {
            guard let data = try await item.loadTransferable(type: Data.self) else {
                if canApplyCurrentResult(requestId: requestId, fingerprint: uploadFingerprint) {
                    onError("Failed to read avatar image.")
                }
                return
            }
            let prepared = try await Task.detached(priority: .utility) {
                try GaryxMobileAvatarImageNormalizer.normalizedDataUrl(fromImageData: data)
            }.value
            guard canApplyCurrentResult(requestId: requestId, fingerprint: uploadFingerprint) else { return }
            avatarDataUrl = prepared
        } catch is CancellationError {
            return
        } catch let error as GaryxMobileAvatarImageNormalizer.NormalizationError {
            if canApplyCurrentResult(requestId: requestId, fingerprint: uploadFingerprint) {
                onError(error.localizedDescription)
            }
        } catch {
            if canApplyCurrentResult(requestId: requestId, fingerprint: uploadFingerprint) {
                onError(error.localizedDescription)
            }
        }
    }

    private func canApplyCurrentResult(requestId: UUID, fingerprint: String) -> Bool {
        !Task.isCancelled && editorState.canApply(requestId: requestId, fingerprint: fingerprint)
    }

    private func finishWork(requestId: UUID) {
        let isCurrentRequest = editorState.requestId == requestId
        editorState.finish(requestId: requestId)
        if isCurrentRequest {
            workTask = nil
        }
    }
}

private struct GaryxAvatarEditorActionLabel: View {
    let title: String
    let systemName: String
    var isLoading = false

    var body: some View {
        HStack(spacing: 7) {
            if isLoading {
                ProgressView()
                    .controlSize(.small)
            } else {
                Image(systemName: systemName)
                    .font(GaryxFont.system(size: 14, weight: .semibold))
            }
            Text(title)
                .font(GaryxFont.footnote(weight: .semibold))
                .lineLimit(1)
        }
        .foregroundStyle(.primary)
        .frame(maxWidth: .infinity)
        .frame(minHeight: 44)
        .padding(.horizontal, 12)
        .background(Color.primary.opacity(0.055), in: Capsule())
        .overlay {
            Capsule()
                .stroke(Color.primary.opacity(0.08), lineWidth: 1)
        }
    }
}

private struct GaryxAvatarStyleSheet: View {
    @Environment(\.dismiss) private var dismiss
    let isGenerating: Bool
    let canGenerate: Bool
    let onGenerate: (String) -> Void
    @State private var selectedStyleId = GaryxAvatarStyleOption.defaultId
    @State private var customStyle = ""

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .center, spacing: 14) {
                Text("Avatar style")
                    .font(GaryxFont.callout(weight: .medium))
                    .foregroundStyle(.primary)
                Spacer(minLength: 0)
                Button {
                    dismiss()
                } label: {
                    GaryxCompactGlassIcon(systemName: "xmark")
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Close")
            }
            .padding(.horizontal, 22)
            .padding(.top, 22)
            .padding(.bottom, 12)

            ScrollView {
                GaryxGlassPanel(cornerRadius: 28, fallbackMaterial: .ultraThinMaterial, shadowOpacity: 0.045) {
                    VStack(spacing: 0) {
                        ForEach(Array(GaryxAvatarStyleOption.builtIn.enumerated()), id: \.element.id) { index, style in
                            GaryxAvatarStyleRow(
                                title: style.label,
                                isSelected: selectedStyleId == style.id
                            ) {
                                selectedStyleId = style.id
                            }
                            if index < GaryxAvatarStyleOption.builtIn.count - 1 {
                                Divider().padding(.leading, 18)
                            }
                        }
                    }
                }
                .padding(.horizontal, 22)
                .padding(.top, 4)

                GaryxGlassPanel(cornerRadius: 28, fallbackMaterial: .ultraThinMaterial, shadowOpacity: 0.045) {
                    VStack(alignment: .leading, spacing: 12) {
                        Button {
                            selectedStyleId = customStyleId
                        } label: {
                            HStack(spacing: 12) {
                                Text("Custom style")
                                    .font(GaryxFont.body(weight: .medium))
                                    .foregroundStyle(.primary)
                                Spacer(minLength: 0)
                                if selectedStyleId == customStyleId {
                                    GaryxSelectionCheckmark(size: 14)
                                }
                            }
                        }
                        .buttonStyle(.plain)

                        TextEditor(text: $customStyle)
                            .font(GaryxFont.callout())
                            .foregroundStyle(.primary)
                            .frame(minHeight: 104)
                            .padding(10)
                            .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                            .overlay(alignment: .topLeading) {
                                if customStyle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                                    Text("e.g. polished paper-cut icon with emerald accents")
                                        .font(GaryxFont.callout())
                                        .foregroundStyle(.tertiary)
                                        .padding(.horizontal, 16)
                                        .padding(.vertical, 18)
                                        .allowsHitTesting(false)
                                }
                            }
                            .onTapGesture {
                                selectedStyleId = customStyleId
                            }
                    }
                    .padding(18)
                }
                .padding(.horizontal, 22)
                .padding(.top, 14)
                .padding(.bottom, 110)
            }
            .scrollIndicators(.hidden)
        }
        .safeAreaInset(edge: .bottom) {
            HStack(spacing: 12) {
                Button {
                    dismiss()
                } label: {
                    Text("Cancel")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(GaryxSecondaryButtonStyle())
                .disabled(isGenerating)

                Button {
                    let prompt = activeStylePrompt
                    dismiss()
                    onGenerate(prompt)
                } label: {
                    HStack(spacing: 8) {
                        if isGenerating {
                            ProgressView()
                                .controlSize(.small)
                        }
                        Text(isGenerating ? "Generating" : "Generate")
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(GaryxPrimaryWideButtonStyle())
                .disabled(!canSubmit || isGenerating)
            }
            .padding(.horizontal, 22)
            .padding(.top, 12)
            .padding(.bottom, 14)
            .background(.regularMaterial)
        }
        .background {
            Rectangle()
                .fill(Color(.systemBackground).opacity(0.98))
                .overlay {
                    LinearGradient(
                        colors: [
                            Color.white.opacity(0.28),
                            Color.white.opacity(0.10)
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                }
                .ignoresSafeArea()
        }
        .presentationBackground(.clear)
        .presentationBackgroundInteraction(.enabled)
        .presentationDetents([.fraction(0.93), .large])
        .presentationDragIndicator(.hidden)
        .presentationCornerRadius(38)
    }

    private var activeStylePrompt: String {
        if selectedStyleId == customStyleId {
            return customStyle.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return GaryxAvatarStyleOption.builtIn.first(where: { $0.id == selectedStyleId })?.prompt
            ?? GaryxAvatarStyleOption.builtIn.first?.prompt
            ?? ""
    }

    private var canSubmit: Bool {
        canGenerate && !activeStylePrompt.isEmpty
    }

    private var customStyleId: String { "custom" }
}

private struct GaryxAvatarStyleRow: View {
    let title: String
    let isSelected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Text(title)
                    .font(GaryxFont.body())
                    .foregroundStyle(.primary)
                Spacer(minLength: 0)
                if isSelected {
                    GaryxSelectionCheckmark(size: 14)
                }
            }
            .padding(.horizontal, 18)
            .frame(minHeight: 54)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

struct GaryxAgentCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let agent: GaryxAgentSummary
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false

    var body: some View {
        GaryxRowActionMenu(actions: agentSwipeActions) {
            Button {
                model.selectedAgentDetail = agent
            } label: {
                GaryxAgentIdentityRow(
                    id: agent.id,
                    title: agent.displayName,
                    subtitle: "",
                    kind: .agent,
                    avatarDataUrl: agent.avatarDataUrl,
                    providerType: agent.providerType,
                    builtIn: agent.builtIn,
                    selected: model.selectedAgentTargetId == agent.id
                )
            }
            .buttonStyle(.plain)
            .contentShape(Rectangle())
        }
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxAgentEditSheet(agent: agent)
        }
        .confirmationDialog("Delete agent?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteAgent(agent) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the custom agent configuration.")
        }
    }

    private var agentSwipeActions: [GaryxRowAction] {
        var actions = [
            GaryxRowAction(title: "Chat", systemImage: "message", tone: .accent) {
                model.openAgentChatDraft(agent.id)
            },
            GaryxRowAction(title: "Use", systemImage: "checkmark.circle") {
                model.setSelectedAgentTarget(agent.id)
            }
        ]
        if !agent.builtIn {
            actions.append(
                GaryxRowAction(title: "Edit", systemImage: "pencil") {
                    showsEditForm = true
                }
            )
            actions.append(
                GaryxRowAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                    showsDeleteConfirmation = true
                }
            )
        }
        return actions
    }
}

struct GaryxTeamCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let team: GaryxTeamSummary
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false

    var body: some View {
        GaryxRowActionMenu(actions: teamSwipeActions) {
            Button {
                model.selectedTeamDetail = team
            } label: {
                GaryxAgentIdentityRow(
                    id: team.id,
                    title: team.displayName,
                    subtitle: team.workflowText,
                    kind: .team,
                    avatarDataUrl: team.avatarDataUrl,
                    providerType: "",
                    selected: model.selectedAgentTargetId == team.id
                )
            }
            .buttonStyle(.plain)
            .contentShape(Rectangle())
        }
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxTeamEditSheet(team: team)
        }
        .confirmationDialog("Delete team?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteTeam(team) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the team configuration.")
        }
    }

    private var teamSwipeActions: [GaryxRowAction] {
        [
            GaryxRowAction(title: "Chat", systemImage: "message", tone: .accent) {
                model.openAgentChatDraft(team.id)
            },
            GaryxRowAction(title: "Use", systemImage: "checkmark.circle") {
                model.setSelectedAgentTarget(team.id)
            },
            GaryxRowAction(title: "Edit", systemImage: "pencil") {
                showsEditForm = true
            },
            GaryxRowAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                showsDeleteConfirmation = true
            }
        ]
    }
}

private struct GaryxAgentProviderOption: Identifiable, Equatable {
    let id: String
    let label: String
}

private let garyxAgentProviderOptions: [GaryxAgentProviderOption] = [
    GaryxAgentProviderOption(id: "claude_code", label: "Claude Code"),
    GaryxAgentProviderOption(id: "codex_app_server", label: "Codex"),
    GaryxAgentProviderOption(id: "gemini_cli", label: "Gemini CLI"),
    GaryxAgentProviderOption(id: "gpt", label: "OpenAI"),
    GaryxAgentProviderOption(id: "anthropic", label: "Anthropic"),
    GaryxAgentProviderOption(id: "google", label: "Google")
]

private struct GaryxAgentProviderSelectionRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Binding var providerType: String
    @Binding var modelName: String
    @Binding var modelReasoningEffort: String
    @State private var showsProviderSheet = false

    var body: some View {
        GaryxFormSelectionRow(
            title: "Provider",
            value: providerLabel,
            placeholder: "Choose provider"
        ) {
            showsProviderSheet = true
        }
        .sheet(isPresented: $showsProviderSheet) {
            GaryxAgentProviderSelectionSheet(
                selectedProvider: normalizedProvider,
                options: providerOptionsIncludingCurrent,
                onSelect: selectProvider
            )
        }
        .task(id: normalizedProvider) {
            await model.loadProviderModels(providerType: normalizedProvider)
        }
    }

    private var normalizedProvider: String {
        providerType.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var providerLabel: String {
        garyxAgentProviderLabel(for: normalizedProvider)
    }

    private var providerOptionsIncludingCurrent: [GaryxAgentProviderOption] {
        guard !normalizedProvider.isEmpty,
              !garyxAgentProviderOptions.contains(where: { $0.id == normalizedProvider }) else {
            return garyxAgentProviderOptions
        }
        return [
            GaryxAgentProviderOption(id: normalizedProvider, label: garyxAgentProviderLabel(for: normalizedProvider))
        ] + garyxAgentProviderOptions
    }

    private func selectProvider(_ nextProvider: String) {
        let previousProvider = normalizedProvider
        providerType = nextProvider
        if previousProvider != nextProvider {
            modelName = ""
            modelReasoningEffort = ""
        }
        Task { await model.loadProviderModels(providerType: nextProvider) }
    }
}

private struct GaryxAgentModelChoice: Identifiable, Equatable {
    let id: String
    let label: String
    let description: String?
    let recommended: Bool
}

private struct GaryxAgentModelSelectionRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Binding var providerType: String
    @Binding var modelName: String
    @State private var showsModelSheet = false

    var body: some View {
        Group {
            if supportsModelMenu {
                GaryxFormSelectionRow(
                    title: "Model",
                    value: selectedModelLabel,
                    placeholder: "Provider default"
                ) {
                    showsModelSheet = true
                }
                .sheet(isPresented: $showsModelSheet) {
                    GaryxAgentModelSelectionSheet(
                        selectedModel: normalizedModel,
                        defaultModel: providerModels?.defaultModel,
                        choices: modelChoices
                    ) { nextModel in
                        modelName = nextModel
                    }
                }
            } else {
                GaryxFormTextFieldRow(
                    title: "Model",
                    text: $modelName,
                    placeholder: "Provider default",
                    autocapitalization: .never,
                    autocorrectionDisabled: true
                )
            }
        }
        .task(id: normalizedProvider) {
            await model.loadProviderModels(providerType: normalizedProvider)
        }
    }

    private var normalizedProvider: String {
        providerType.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var normalizedModel: String {
        modelName.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var providerModels: GaryxProviderModels? {
        model.providerModelsByType[normalizedProvider]
    }

    private var supportsModelMenu: Bool {
        providerModels?.supportsModelSelection == true && !modelChoices.isEmpty
    }

    private var modelChoices: [GaryxAgentModelChoice] {
        var choices = providerModels?.models.map {
            GaryxAgentModelChoice(
                id: $0.id,
                label: $0.label,
                description: $0.description,
                recommended: $0.recommended
            )
        } ?? []
        if !normalizedModel.isEmpty, !choices.contains(where: { $0.id == normalizedModel }) {
            choices.insert(
                GaryxAgentModelChoice(
                    id: normalizedModel,
                    label: normalizedModel,
                    description: nil,
                    recommended: false
                ),
                at: 0
            )
        }
        return choices.filter { !$0.id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    private var selectedModelLabel: String {
        guard !normalizedModel.isEmpty else {
            if let defaultModel = providerModels?.defaultModel?.trimmingCharacters(in: .whitespacesAndNewlines),
               !defaultModel.isEmpty {
                return "Default: \(defaultModel)"
            }
            return "Provider default"
        }
        return modelChoices.first(where: { $0.id == normalizedModel })?.label ?? normalizedModel
    }

    private func modelChoiceTitle(_ choice: GaryxAgentModelChoice) -> String {
        choice.recommended ? "\(choice.label) · Recommended" : choice.label
    }
}

private struct GaryxAgentReasoningEffortSelectionRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Binding var providerType: String
    @Binding var modelName: String
    @Binding var reasoningEffort: String
    @State private var showsEffortSheet = false

    var body: some View {
        if !effortChoices.isEmpty {
            Divider().padding(.leading, 16)
            GaryxFormSelectionRow(
                title: "Thinking level",
                value: selectedEffortLabel,
                placeholder: "Provider default"
            ) {
                showsEffortSheet = true
            }
            .sheet(isPresented: $showsEffortSheet) {
                GaryxAgentReasoningEffortSelectionSheet(
                    selectedEffort: normalizedEffort,
                    choices: effortChoices
                ) { nextEffort in
                    reasoningEffort = nextEffort
                }
            }
            .onChange(of: modelName) { _, _ in
                // Drop a thinking level the newly selected model does not support.
                reasoningEffort = GaryxThreadModelOverridePresentation.sanitizedReasoningEffort(
                    providerModels: providerModels,
                    model: modelName,
                    reasoningEffort: reasoningEffort
                ) ?? ""
            }
        }
    }

    private var normalizedProvider: String {
        providerType.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var normalizedEffort: String {
        reasoningEffort.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var providerModels: GaryxProviderModels? {
        model.providerModelsByType[normalizedProvider]
    }

    private var effortChoices: [GaryxProviderModelOption] {
        GaryxThreadModelOverridePresentation.reasoningEffortOptions(
            providerModels: providerModels,
            model: modelName
        )
    }

    private var selectedEffortLabel: String {
        guard !normalizedEffort.isEmpty else { return "Provider default" }
        return effortChoices.first(where: { $0.id == normalizedEffort })?.label ?? normalizedEffort
    }
}

private struct GaryxAgentReasoningEffortSelectionSheet: View {
    @Environment(\.dismiss) private var dismiss
    let selectedEffort: String
    let choices: [GaryxProviderModelOption]
    let onSelect: (String) -> Void

    var body: some View {
        GaryxAgentOptionSelectionSheet(title: "Thinking level", subtitle: "Choose thinking level") {
            GaryxAgentPlainOptionRow(
                title: "Provider default",
                subtitle: "Use the model's default thinking level",
                systemName: "wand.and.stars",
                selected: selectedEffort.isEmpty
            ) {
                onSelect("")
                dismiss()
            }

            if !choices.isEmpty {
                Divider().padding(.leading, 58)
            }

            ForEach(Array(choices.enumerated()), id: \.element.id) { index, choice in
                GaryxAgentPlainOptionRow(
                    title: choice.recommended ? "\(choice.label) · Recommended" : choice.label,
                    subtitle: choice.description ?? choice.id,
                    systemName: "brain",
                    selected: selectedEffort == choice.id
                ) {
                    onSelect(choice.id)
                    dismiss()
                }
                if index < choices.count - 1 {
                    Divider().padding(.leading, 58)
                }
            }
        }
    }
}

private struct GaryxAgentProviderSelectionSheet: View {
    @Environment(\.dismiss) private var dismiss
    let selectedProvider: String
    let options: [GaryxAgentProviderOption]
    let onSelect: (String) -> Void

    var body: some View {
        GaryxAgentOptionSelectionSheet(title: "Provider", subtitle: "Choose provider") {
            if options.isEmpty {
                GaryxAgentOptionEmptyState(text: "No providers available.")
            } else {
                ForEach(Array(options.enumerated()), id: \.element.id) { index, option in
                    GaryxAgentPlainOptionRow(
                        title: option.label,
                        subtitle: option.id,
                        systemName: "server.rack",
                        selected: selectedProvider == option.id
                    ) {
                        onSelect(option.id)
                        dismiss()
                    }
                    if index < options.count - 1 {
                        Divider().padding(.leading, 58)
                    }
                }
            }
        }
    }
}

private struct GaryxAgentModelSelectionSheet: View {
    @Environment(\.dismiss) private var dismiss
    let selectedModel: String
    let defaultModel: String?
    let choices: [GaryxAgentModelChoice]
    let onSelect: (String) -> Void

    var body: some View {
        GaryxAgentOptionSelectionSheet(title: "Model", subtitle: "Choose model") {
            GaryxAgentPlainOptionRow(
                title: "Provider default",
                subtitle: defaultModelSubtitle,
                systemName: "wand.and.stars",
                selected: selectedModel.isEmpty
            ) {
                onSelect("")
                dismiss()
            }

            if !choices.isEmpty {
                Divider().padding(.leading, 58)
            }

            ForEach(Array(choices.enumerated()), id: \.element.id) { index, choice in
                GaryxAgentPlainOptionRow(
                    title: modelChoiceTitle(choice),
                    subtitle: choice.description ?? choice.id,
                    systemName: "cube",
                    selected: selectedModel == choice.id
                ) {
                    onSelect(choice.id)
                    dismiss()
                }
                if index < choices.count - 1 {
                    Divider().padding(.leading, 58)
                }
            }
        }
    }

    private var defaultModelSubtitle: String {
        let trimmed = defaultModel?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? "Use provider default model" : "Default: \(trimmed)"
    }

    private func modelChoiceTitle(_ choice: GaryxAgentModelChoice) -> String {
        choice.recommended ? "\(choice.label) · Recommended" : choice.label
    }
}

private struct GaryxTeamLeaderSelectionSheet: View {
    @Environment(\.dismiss) private var dismiss
    let selectedAgentId: String
    let agents: [GaryxAgentSummary]
    let onSelect: (String) -> Void

    var body: some View {
        GaryxAgentOptionSelectionSheet(title: "Leader", subtitle: "Choose team leader") {
            if agents.isEmpty {
                GaryxAgentOptionEmptyState(text: "No agents available.")
            } else {
                ForEach(Array(agents.enumerated()), id: \.element.id) { index, agent in
                    GaryxAgentSummaryOptionRow(
                        agent: agent,
                        selected: selectedAgentId == agent.id
                    ) {
                        onSelect(agent.id)
                        dismiss()
                    }
                    if index < agents.count - 1 {
                        Divider().padding(.leading, 62)
                    }
                }
            }
        }
    }
}

private struct GaryxAgentOptionSelectionSheet<Content: View>: View {
    @Environment(\.dismiss) private var dismiss
    let title: String
    let subtitle: String
    let content: Content

    init(title: String, subtitle: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.subtitle = subtitle
        self.content = content()
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .font(GaryxFont.callout(weight: .semibold))
                        .foregroundStyle(.primary)
                    Text(subtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
                Button {
                    dismiss()
                } label: {
                    GaryxCompactGlassIcon(systemName: "xmark")
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Close")
            }
            .padding(.horizontal, 22)
            .padding(.top, 22)
            .padding(.bottom, 14)

            ScrollView {
                GaryxGlassPanel(cornerRadius: 28, fallbackMaterial: .ultraThinMaterial, shadowOpacity: 0.045) {
                    VStack(spacing: 0) {
                        content
                    }
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                }
                .padding(.horizontal, 22)
                .padding(.bottom, 28)
            }
            .scrollIndicators(.hidden)
        }
        .background(Color(.systemBackground).opacity(0.98).ignoresSafeArea())
        .presentationDetents([.fraction(0.93), .large])
        .presentationDragIndicator(.hidden)
        .presentationCornerRadius(34)
    }
}

private struct GaryxAgentOptionEmptyState: View {
    let text: String

    var body: some View {
        Text(text)
            .font(GaryxFont.subheadline())
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 28)
    }
}

private struct GaryxAgentPlainOptionRow: View {
    let title: String
    let subtitle: String
    let systemName: String
    let selected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: systemName)
                    .font(GaryxFont.system(size: 16, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 34, height: 34)
                    .background(Color(.tertiarySystemFill).opacity(0.72), in: Circle())

                VStack(alignment: .leading, spacing: 3) {
                    Text(title)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    if !subtitle.isEmpty {
                        Text(subtitle)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                }

                Spacer(minLength: 0)

                if selected {
                    GaryxSelectionCheckmark(size: 13)
                        .frame(width: 22, height: 28)
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, minHeight: 54, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

private struct GaryxAgentSummaryOptionRow: View {
    let agent: GaryxAgentSummary
    let selected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                GaryxAgentAvatarView(
                    agentId: agent.id,
                    avatarDataUrl: agent.avatarDataUrl,
                    kind: .agent,
                    label: agent.displayName,
                    providerType: agent.providerType,
                    builtIn: agent.builtIn,
                    diameter: 32
                )

                VStack(alignment: .leading, spacing: 3) {
                    Text(agent.displayName)
                        .font(GaryxFont.subheadline(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Text(agent.id)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                Spacer(minLength: 0)

                if selected {
                    GaryxSelectionCheckmark(size: 13)
                        .frame(width: 22, height: 28)
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, minHeight: 54, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

private struct GaryxTeamLeaderSelectionRow: View {
    @Binding var leaderAgentId: String
    @Binding var memberAgentIds: String
    let agents: [GaryxAgentSummary]
    @State private var showsLeaderSheet = false

    var body: some View {
        GaryxFormSelectionRow(
            title: "Leader",
            value: agentOptionsIncludingCurrent.isEmpty ? "" : leaderLabel,
            placeholder: agentOptionsIncludingCurrent.isEmpty ? "No agents" : "Choose leader"
        ) {
            if !agentOptionsIncludingCurrent.isEmpty {
                showsLeaderSheet = true
            }
        }
        .disabled(agentOptionsIncludingCurrent.isEmpty)
        .sheet(isPresented: $showsLeaderSheet) {
            GaryxTeamLeaderSelectionSheet(
                selectedAgentId: normalizedLeader,
                agents: agentOptionsIncludingCurrent,
                onSelect: selectLeader
            )
        }
    }

    private var normalizedLeader: String {
        leaderAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var agentOptionsIncludingCurrent: [GaryxAgentSummary] {
        garyxTeamAgentOptions(agents, preserving: [normalizedLeader])
    }

    private var leaderLabel: String {
        guard !normalizedLeader.isEmpty else { return "Choose leader" }
        return agentOptionsIncludingCurrent.first(where: { $0.id == normalizedLeader })?.displayName ?? normalizedLeader
    }

    private func selectLeader(_ agentId: String) {
        leaderAgentId = agentId
        var members = garyxTeamMemberIds(from: memberAgentIds)
        if !members.contains(agentId) {
            members.insert(agentId, at: 0)
        }
        memberAgentIds = garyxTeamMemberIdsString(members)
    }
}

private struct GaryxTeamMembersSelectionRow: View {
    @Binding var leaderAgentId: String
    @Binding var memberAgentIds: String
    let agents: [GaryxAgentSummary]
    @State private var showsMembersSheet = false

    var body: some View {
        GaryxFormSelectionRow(
            title: "Members",
            value: membersLabel,
            placeholder: "Choose members"
        ) {
            showsMembersSheet = true
        }
        .sheet(isPresented: $showsMembersSheet) {
            GaryxTeamMembersSelectionSheet(
                leaderAgentId: $leaderAgentId,
                memberAgentIds: $memberAgentIds,
                agents: agents
            )
        }
    }

    private var membersLabel: String {
        garyxTeamMembersLabel(memberIds: garyxTeamMemberIds(from: memberAgentIds), agents: agents)
    }
}

private struct GaryxTeamMembersSelectionSheet: View {
    @Environment(\.dismiss) private var dismiss
    @Binding var leaderAgentId: String
    @Binding var memberAgentIds: String
    let agents: [GaryxAgentSummary]

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Team Members")
                        .font(GaryxFont.callout(weight: .semibold))
                        .foregroundStyle(.primary)
                    Text("\(selectedIds.count) selected")
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
                Button {
                    dismiss()
                } label: {
                    GaryxCompactGlassIcon(systemName: "xmark")
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Close")
            }
            .padding(.horizontal, 22)
            .padding(.top, 22)
            .padding(.bottom, 14)

            ScrollView {
                GaryxGlassPanel(cornerRadius: 28, fallbackMaterial: .ultraThinMaterial, shadowOpacity: 0.045) {
                    VStack(spacing: 0) {
                        if agentOptionsIncludingCurrent.isEmpty {
                            Text("No agents available.")
                                .font(GaryxFont.subheadline())
                                .foregroundStyle(.secondary)
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 28)
                        } else {
                            ForEach(Array(agentOptionsIncludingCurrent.enumerated()), id: \.element.id) { index, agent in
                                teamMemberRow(agent)
                                if index < agentOptionsIncludingCurrent.count - 1 {
                                    Divider().padding(.leading, 62)
                                }
                            }
                        }
                    }
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                }
                .padding(.horizontal, 22)
                .padding(.bottom, 28)
            }
            .scrollIndicators(.hidden)
        }
        .background(Color(.systemBackground).opacity(0.98).ignoresSafeArea())
        .presentationDetents([.fraction(0.86), .large])
        .presentationDragIndicator(.hidden)
        .presentationCornerRadius(34)
    }

    private var normalizedLeader: String {
        leaderAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var selectedIds: [String] {
        garyxTeamMemberIds(from: memberAgentIds)
    }

    private var agentOptionsIncludingCurrent: [GaryxAgentSummary] {
        garyxTeamAgentOptions(agents, preserving: selectedIds + [normalizedLeader])
    }

    private func teamMemberRow(_ agent: GaryxAgentSummary) -> some View {
        let selected = selectedIds.contains(agent.id)
        let isLeader = normalizedLeader == agent.id
        return HStack(spacing: 12) {
            Button {
                toggleMember(agent.id)
            } label: {
                HStack(spacing: 12) {
                    GaryxAgentAvatarView(
                        agentId: agent.id,
                        avatarDataUrl: agent.avatarDataUrl,
                        kind: .agent,
                        label: agent.displayName,
                        providerType: agent.providerType,
                        builtIn: agent.builtIn,
                        diameter: 32
                    )
                    VStack(alignment: .leading, spacing: 3) {
                        HStack(spacing: 6) {
                            Text(agent.displayName)
                                .font(GaryxFont.subheadline(weight: .semibold))
                                .foregroundStyle(.primary)
                                .lineLimit(1)
                            if isLeader {
                                Text("TL")
                                    .font(GaryxFont.caption(weight: .bold))
                                    .foregroundStyle(.secondary)
                            }
                        }
                        Text(agent.id)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if selected {
                Button {
                    leaderAgentId = agent.id
                    if !selectedIds.contains(agent.id) {
                        memberAgentIds = garyxTeamMemberIdsString([agent.id] + selectedIds)
                    }
                } label: {
                    Text(isLeader ? "Lead" : "Set TL")
                        .font(GaryxFont.caption(weight: .semibold))
                        .foregroundStyle(.primary)
                        .padding(.horizontal, 8)
                        .frame(height: 28)
                        .background(Color(.tertiarySystemFill).opacity(0.72), in: Capsule())
                }
                .buttonStyle(.plain)
            }

            Button {
                toggleMember(agent.id)
            } label: {
                if selected {
                    GaryxSelectionCheckmark(size: 13)
                        .frame(width: 22, height: 28)
                } else {
                    Image(systemName: "plus")
                        .font(GaryxFont.system(size: 13, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 22, height: 28)
                }
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity, minHeight: 54, alignment: .leading)
    }

    private func toggleMember(_ agentId: String) {
        var nextIds = selectedIds
        if nextIds.contains(agentId) {
            nextIds.removeAll { $0 == agentId }
            if normalizedLeader == agentId {
                leaderAgentId = nextIds.first ?? ""
            }
        } else {
            nextIds.append(agentId)
            if normalizedLeader.isEmpty {
                leaderAgentId = agentId
            }
        }
        memberAgentIds = garyxTeamMemberIdsString(nextIds)
    }
}

private func garyxAgentProviderLabel(for providerType: String) -> String {
    let normalized = providerType.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !normalized.isEmpty else { return "Choose provider" }
    return garyxAgentProviderOptions.first(where: { $0.id == normalized })?.label
        ?? GaryxProviderPresentation.displayName(for: normalized)
}

private func garyxTeamAgentOptions(
    _ agents: [GaryxAgentSummary],
    preserving ids: [String]
) -> [GaryxAgentSummary] {
    var seen = Set<String>()
    var result = agents
        .filter(\.standalone)
        .sorted { left, right in
            if left.builtIn != right.builtIn {
                return left.builtIn && !right.builtIn
            }
            return left.displayName.localizedCaseInsensitiveCompare(right.displayName) == .orderedAscending
        }
        .filter { seen.insert($0.id).inserted }
    for id in ids {
        let trimmed = id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !seen.contains(trimmed) else { continue }
        result.insert(
            GaryxAgentSummary(
                id: trimmed,
                displayName: trimmed,
                providerType: "",
                model: "",
                builtIn: false,
                standalone: true
            ),
            at: 0
        )
        seen.insert(trimmed)
    }
    return result
}

private func garyxTeamMemberIds(from value: String) -> [String] {
    var seen = Set<String>()
    return value
        .split { $0 == "," || $0 == "\n" || $0 == " " }
        .map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) }
        .filter { !$0.isEmpty && seen.insert($0).inserted }
}

private func garyxTeamMemberIdsString(_ ids: [String]) -> String {
    ids
        .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
        .filter { !$0.isEmpty }
        .joined(separator: ", ")
}

private func garyxTeamMembersLabel(memberIds: [String], agents: [GaryxAgentSummary]) -> String {
    guard !memberIds.isEmpty else { return "" }
    let namesById = Dictionary(uniqueKeysWithValues: agents.map { ($0.id, $0.displayName) })
    let names = memberIds.map { namesById[$0] ?? $0 }
    if names.count <= 2 {
        return names.joined(separator: ", ")
    }
    return "\(names[0]), \(names[1]) +\(names.count - 2)"
}
