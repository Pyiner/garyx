import Foundation
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
    }
}

struct GaryxCreateAgentCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        GaryxFormSheet(
            title: "New Agent",
            canSave: canCreate,
            onSave: { Task { await createAgent() } }
        ) {
            VStack(alignment: .leading, spacing: 22) {
                GaryxFormGroupedSection(title: "Identity") {
                    TextField("Agent ID", text: $model.draftAgentId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxFormTextField()
                    Divider().padding(.leading, 16)
                    TextField("Display name", text: $model.draftAgentName)
                        .garyxFormTextField()
                }

                GaryxFormGroupedSection(title: "Model") {
                    TextField("Provider", text: $model.draftAgentProvider)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxFormTextField()
                    Divider().padding(.leading, 16)
                    TextField("Model", text: $model.draftAgentModel)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxFormTextField()
                }

                GaryxFormGroupedSection(title: "Defaults") {
                    GaryxWorkspacePathSelectionRow(
                        title: "Default workspace",
                        path: $model.draftAgentWorkspace,
                        workspacePaths: model.userWorkspacePaths,
                        placeholder: "Optional",
                        allowsEmpty: true
                    )
                    Divider().padding(.leading, 16)
                    TextField("System Prompt", text: $model.draftAgentPrompt, axis: .vertical)
                        .lineLimit(2...6)
                        .garyxFormTextArea()
                }
            }
        }
    }

    private var canCreate: Bool {
        !model.draftAgentId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !model.draftAgentProvider.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func createAgent() async {
        guard canCreate else { return }
        if await model.createAgentFromDraft() {
            dismiss()
        }
    }
}

struct GaryxCreateTeamCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        GaryxFormSheet(
            title: "New Team",
            canSave: canCreate,
            onSave: { Task { await createTeam() } }
        ) {
            VStack(alignment: .leading, spacing: 22) {
                GaryxFormGroupedSection(title: "Identity") {
                    TextField("Team ID", text: $model.draftTeamId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxFormTextField()
                    Divider().padding(.leading, 16)
                    TextField("Display name", text: $model.draftTeamName)
                        .garyxFormTextField()
                }

                GaryxFormGroupedSection(title: "Members") {
                    TextField("Leader Agent", text: $model.draftTeamLeaderId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxFormTextField()
                    Divider().padding(.leading, 16)
                    TextField("Members", text: $model.draftTeamMemberIds)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxFormTextField()
                }

                GaryxFormGroupedSection(title: "Workflow") {
                    TextField("Workflow", text: $model.draftTeamWorkflow, axis: .vertical)
                        .lineLimit(2...6)
                        .garyxFormTextArea()
                }
            }
        }
    }

    private var canCreate: Bool {
        !model.draftTeamId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func createTeam() async {
        guard canCreate else { return }
        if await model.createTeamFromDraft() {
            dismiss()
        }
    }
}

struct GaryxAgentCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let agent: GaryxAgentSummary
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false
    @State private var agentId = ""
    @State private var displayName = ""
    @State private var providerType = ""
    @State private var modelName = ""
    @State private var workspace = ""
    @State private var systemPrompt = ""

    var body: some View {
        GaryxRowActionMenu(actions: agentSwipeActions) {
            Button {
                if agent.builtIn {
                    model.setSelectedAgentTarget(agent.id)
                } else {
                    fillDraft()
                    showsEditForm = true
                }
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
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(
                title: "Edit Agent",
                canSave: canSaveAgent,
                onSave: { Task { await saveAgent() } }
            ) {
                agentFormFields
            }
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
                    fillDraft()
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

    private func fillDraft() {
        agentId = agent.id
        displayName = agent.displayName
        providerType = agent.providerType
        modelName = agent.model
        workspace = agent.defaultWorkspaceDir
        systemPrompt = agent.systemPrompt
    }

    private var agentFormFields: some View {
        VStack(alignment: .leading, spacing: 22) {
            GaryxFormGroupedSection(title: "Identity") {
                TextField("Agent ID", text: $agentId)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .garyxFormTextField()
                Divider().padding(.leading, 16)
                TextField("Display name", text: $displayName)
                    .garyxFormTextField()
            }

            GaryxFormGroupedSection(title: "Model") {
                TextField("Provider", text: $providerType)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .garyxFormTextField()
                Divider().padding(.leading, 16)
                TextField("Model", text: $modelName)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .garyxFormTextField()
            }

            GaryxFormGroupedSection(title: "Defaults") {
                GaryxWorkspacePathSelectionRow(
                    title: "Default workspace",
                    path: $workspace,
                    workspacePaths: model.userWorkspacePaths,
                    placeholder: "Optional",
                    allowsEmpty: true
                )
                Divider().padding(.leading, 16)
                TextField("System Prompt", text: $systemPrompt, axis: .vertical)
                    .lineLimit(2...6)
                    .garyxFormTextArea()
            }
        }
    }

    private var canSaveAgent: Bool {
        !agentId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !providerType.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func saveAgent() async {
        guard canSaveAgent else { return }
        await model.updateAgent(
            agent,
            agentId: agentId,
            displayName: displayName,
            providerType: providerType,
            modelName: modelName,
            workspace: workspace,
            systemPrompt: systemPrompt
        )
        showsEditForm = false
    }
}

struct GaryxTeamCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let team: GaryxTeamSummary
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false
    @State private var teamId = ""
    @State private var displayName = ""
    @State private var leaderAgentId = ""
    @State private var memberAgentIds = ""
    @State private var workflowText = ""

    var body: some View {
        GaryxRowActionMenu(actions: teamSwipeActions) {
            Button {
                fillDraft()
                showsEditForm = true
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
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(
                title: "Edit Team",
                canSave: canSaveTeam,
                onSave: { Task { await saveTeam() } }
            ) {
                teamFormFields
            }
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
                fillDraft()
                showsEditForm = true
            },
            GaryxRowAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                showsDeleteConfirmation = true
            }
        ]
    }

    private func fillDraft() {
        teamId = team.id
        displayName = team.displayName
        leaderAgentId = team.leaderAgentId
        memberAgentIds = team.memberAgentIds.joined(separator: ", ")
        workflowText = team.workflowText
    }

    private var teamFormFields: some View {
        VStack(alignment: .leading, spacing: 22) {
            GaryxFormGroupedSection(title: "Identity") {
                TextField("Team ID", text: $teamId)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .garyxFormTextField()
                Divider().padding(.leading, 16)
                TextField("Display name", text: $displayName)
                    .garyxFormTextField()
            }

            GaryxFormGroupedSection(title: "Members") {
                TextField("Leader Agent", text: $leaderAgentId)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .garyxFormTextField()
                Divider().padding(.leading, 16)
                TextField("Members", text: $memberAgentIds)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .garyxFormTextField()
            }

            GaryxFormGroupedSection(title: "Workflow") {
                TextField("Workflow", text: $workflowText, axis: .vertical)
                    .lineLimit(2...6)
                    .garyxFormTextArea()
            }
        }
    }

    private var canSaveTeam: Bool {
        !teamId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func saveTeam() async {
        guard canSaveTeam else { return }
        await model.updateTeam(
            team,
            teamId: teamId,
            displayName: displayName,
            leaderAgentId: leaderAgentId,
            memberAgentIds: memberAgentIds,
            workflowText: workflowText
        )
        showsEditForm = false
    }
}
