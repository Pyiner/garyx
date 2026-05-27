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
    let agent: GaryxAgentSummary

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            GaryxFormGroupedSection(title: "Agent") {
                VStack(spacing: 0) {
                    GaryxAgentDetailInfoRow(title: "Name", value: agent.displayName)
                    Divider().padding(.leading, 16)
                    GaryxAgentDetailInfoRow(title: "ID", value: agent.id)
                    Divider().padding(.leading, 16)
                    GaryxAgentDetailInfoRow(title: "Type", value: agent.builtIn ? "Built-in" : "Custom")
                    Divider().padding(.leading, 16)
                    GaryxAgentDetailInfoRow(title: "Provider", value: agent.providerType)
                    Divider().padding(.leading, 16)
                    GaryxAgentDetailInfoRow(title: "Model", value: agent.model.isEmpty ? "Default" : agent.model)
                    if !agent.defaultWorkspaceDir.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        Divider().padding(.leading, 16)
                        GaryxAgentDetailInfoRow(title: "Workspace", value: agent.defaultWorkspaceDir)
                    }
                }
            }

            if !agent.systemPrompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                GaryxFormGroupedSection(title: "System Prompt") {
                    Text(agent.systemPrompt)
                        .font(GaryxFont.callout())
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(16)
                }
            }
        }
    }
}

struct GaryxTeamDetailCard: View {
    let team: GaryxTeamSummary

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            GaryxFormGroupedSection(title: "Team") {
                VStack(spacing: 0) {
                    GaryxAgentDetailInfoRow(title: "Name", value: team.displayName)
                    Divider().padding(.leading, 16)
                    GaryxAgentDetailInfoRow(title: "ID", value: team.id)
                    Divider().padding(.leading, 16)
                    GaryxAgentDetailInfoRow(title: "Leader", value: team.leaderAgentId)
                    Divider().padding(.leading, 16)
                    GaryxAgentDetailInfoRow(
                        title: "Members",
                        value: team.memberAgentIds.isEmpty ? "No members" : team.memberAgentIds.joined(separator: ", ")
                    )
                }
            }

            if !team.workflowText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                GaryxFormGroupedSection(title: "Workflow") {
                    Text(team.workflowText)
                        .font(GaryxFont.callout())
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(16)
                }
            }
        }
    }
}

private struct GaryxAgentDetailInfoRow: View {
    let title: String
    let value: String

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Text(title)
                .font(GaryxFont.body())
                .foregroundStyle(.primary)
                .frame(width: 92, alignment: .leading)
            Text(value.isEmpty ? "None" : value)
                .font(GaryxFont.body())
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(16)
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
                GaryxAvatarEditorSection(
                    kind: .agent,
                    identifier: model.draftAgentId,
                    displayName: model.draftAgentName,
                    providerType: model.draftAgentProvider,
                    avatarDataUrl: $model.draftAgentAvatarDataUrl
                ) { stylePrompt in
                    await model.generateAvatar(
                        kind: .agent,
                        identifier: model.draftAgentId,
                        displayName: model.draftAgentName,
                        stylePrompt: stylePrompt
                    )
                } onError: { message in
                    model.lastError = message
                }

                GaryxFormGroupedSection(title: "Identity") {
                    GaryxFormTextFieldRow(
                        title: "Agent ID",
                        text: $model.draftAgentId,
                        autocapitalization: .never,
                        autocorrectionDisabled: true
                    )
                    Divider().padding(.leading, 16)
                    GaryxFormTextFieldRow(
                        title: "Display name",
                        text: $model.draftAgentName,
                        placeholder: "Optional"
                    )
                }

                GaryxFormGroupedSection(title: "Model") {
                    GaryxAgentProviderSelectionRow(
                        providerType: $model.draftAgentProvider,
                        modelName: $model.draftAgentModel
                    )
                    Divider().padding(.leading, 16)
                    GaryxAgentModelSelectionRow(
                        providerType: $model.draftAgentProvider,
                        modelName: $model.draftAgentModel
                    )
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
                    GaryxFormTextAreaRow(
                        title: "System Prompt",
                        text: $model.draftAgentPrompt,
                        minHeight: 132,
                        lineLimits: 2...6
                    )
                }
            }
        }
    }

    private var canCreate: Bool {
        !model.draftAgentId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !model.draftAgentName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
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
                GaryxAvatarEditorSection(
                    kind: .team,
                    identifier: model.draftTeamId,
                    displayName: model.draftTeamName,
                    providerType: "",
                    avatarDataUrl: $model.draftTeamAvatarDataUrl
                ) { stylePrompt in
                    await model.generateAvatar(
                        kind: .team,
                        identifier: model.draftTeamId,
                        displayName: model.draftTeamName,
                        stylePrompt: stylePrompt
                    )
                } onError: { message in
                    model.lastError = message
                }

                GaryxFormGroupedSection(title: "Identity") {
                    GaryxFormTextFieldRow(
                        title: "Team ID",
                        text: $model.draftTeamId,
                        autocapitalization: .never,
                        autocorrectionDisabled: true
                    )
                    Divider().padding(.leading, 16)
                    GaryxFormTextFieldRow(
                        title: "Display name",
                        text: $model.draftTeamName,
                        placeholder: "Optional"
                    )
                }

                GaryxFormGroupedSection(title: "Members") {
                    GaryxTeamLeaderSelectionRow(
                        leaderAgentId: $model.draftTeamLeaderId,
                        memberAgentIds: $model.draftTeamMemberIds,
                        agents: model.agents
                    )
                    Divider().padding(.leading, 16)
                    GaryxTeamMembersSelectionRow(
                        leaderAgentId: $model.draftTeamLeaderId,
                        memberAgentIds: $model.draftTeamMemberIds,
                        agents: model.agents
                    )
                }

                GaryxFormGroupedSection(title: "Workflow") {
                    GaryxFormTextAreaRow(
                        title: "Workflow",
                        text: $model.draftTeamWorkflow,
                        minHeight: 132,
                        lineLimits: 2...6
                    )
                }
            }
        }
    }

    private var canCreate: Bool {
        !model.draftTeamId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !model.draftTeamName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !model.draftTeamLeaderId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func createTeam() async {
        guard canCreate else { return }
        if await model.createTeamFromDraft() {
            dismiss()
        }
    }
}

private struct GaryxAvatarEditorSection: View {
    let kind: GaryxAgentAvatarKind
    let identifier: String
    let displayName: String
    let providerType: String
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

        if hasAvatar {
            Button {
                avatarDataUrl = ""
                editorState.reset()
            } label: {
                GaryxAvatarEditorActionLabel(title: "Clear", systemName: "xmark.circle")
            }
            .buttonStyle(.plain)
            .disabled(editorState.isBusy)
            .accessibilityLabel("Clear avatar")
        }
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

    private var hasAvatar: Bool {
        !avatarDataUrl.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
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
        showsStyleSheet = false
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
                    onGenerate(activeStylePrompt)
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
    @State private var agentId = ""
    @State private var displayName = ""
    @State private var providerType = ""
    @State private var modelName = ""
    @State private var workspace = ""
    @State private var avatarDataUrl = ""
    @State private var systemPrompt = ""

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
        avatarDataUrl = agent.avatarDataUrl
        systemPrompt = agent.systemPrompt
    }

    private var agentFormFields: some View {
        VStack(alignment: .leading, spacing: 22) {
            GaryxAvatarEditorSection(
                kind: .agent,
                identifier: agentId,
                displayName: displayName,
                providerType: providerType,
                avatarDataUrl: $avatarDataUrl
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

            GaryxFormGroupedSection(title: "Identity") {
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
            }

            GaryxFormGroupedSection(title: "Model") {
                GaryxAgentProviderSelectionRow(
                    providerType: $providerType,
                    modelName: $modelName
                )
                Divider().padding(.leading, 16)
                GaryxAgentModelSelectionRow(
                    providerType: $providerType,
                    modelName: $modelName
                )
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
                GaryxFormTextAreaRow(
                    title: "System Prompt",
                    text: $systemPrompt,
                    minHeight: 132,
                    lineLimits: 2...6
                )
            }
        }
    }

    private var canSaveAgent: Bool {
        !agentId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
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
            avatarDataUrl: avatarDataUrl,
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
    @State private var avatarDataUrl = ""
    @State private var leaderAgentId = ""
    @State private var memberAgentIds = ""
    @State private var workflowText = ""

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
        avatarDataUrl = team.avatarDataUrl
        leaderAgentId = team.leaderAgentId
        memberAgentIds = team.memberAgentIds.joined(separator: ", ")
        workflowText = team.workflowText
    }

    private var teamFormFields: some View {
        VStack(alignment: .leading, spacing: 22) {
            GaryxAvatarEditorSection(
                kind: .team,
                identifier: teamId,
                displayName: displayName,
                providerType: "",
                avatarDataUrl: $avatarDataUrl
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

            GaryxFormGroupedSection(title: "Identity") {
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
            }

            GaryxFormGroupedSection(title: "Members") {
                GaryxTeamLeaderSelectionRow(
                    leaderAgentId: $leaderAgentId,
                    memberAgentIds: $memberAgentIds,
                    agents: model.agents
                )
                Divider().padding(.leading, 16)
                GaryxTeamMembersSelectionRow(
                    leaderAgentId: $leaderAgentId,
                    memberAgentIds: $memberAgentIds,
                    agents: model.agents
                )
            }

            GaryxFormGroupedSection(title: "Workflow") {
                GaryxFormTextAreaRow(
                    title: "Workflow",
                    text: $workflowText,
                    minHeight: 132,
                    lineLimits: 2...6
                )
            }
        }
    }

    private var canSaveTeam: Bool {
        !teamId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !leaderAgentId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func saveTeam() async {
        guard canSaveTeam else { return }
        await model.updateTeam(
            team,
            teamId: teamId,
            displayName: displayName,
            leaderAgentId: leaderAgentId,
            memberAgentIds: memberAgentIds,
            workflowText: workflowText,
            avatarDataUrl: avatarDataUrl
        )
        showsEditForm = false
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

    var body: some View {
        GaryxFormRow(title: "Provider") {
            Menu {
                ForEach(providerOptionsIncludingCurrent) { option in
                    Button {
                        selectProvider(option.id)
                    } label: {
                        GaryxMenuSelectionLabel(
                            title: option.label,
                            selected: normalizedProvider == option.id,
                            fallbackSystemImage: "server.rack"
                        )
                    }
                }
            } label: {
                GaryxFormMenuValueLabel(value: providerLabel)
            }
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

    var body: some View {
        Group {
            if supportsModelMenu {
                GaryxFormRow(title: "Model") {
                    Menu {
                        Button {
                            modelName = ""
                        } label: {
                            GaryxMenuSelectionLabel(
                                title: "Provider default",
                                selected: normalizedModel.isEmpty,
                                fallbackSystemImage: "wand.and.stars"
                            )
                        }

                        ForEach(modelChoices) { choice in
                            Button {
                                modelName = choice.id
                            } label: {
                                GaryxMenuSelectionLabel(
                                    title: modelChoiceTitle(choice),
                                    selected: normalizedModel == choice.id,
                                    fallbackSystemImage: "cube"
                                )
                            }
                        }
                    } label: {
                        GaryxFormMenuValueLabel(value: selectedModelLabel)
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

private struct GaryxTeamLeaderSelectionRow: View {
    @Binding var leaderAgentId: String
    @Binding var memberAgentIds: String
    let agents: [GaryxAgentSummary]

    var body: some View {
        GaryxFormRow(title: "Leader") {
            if agentOptionsIncludingCurrent.isEmpty {
                Text("No agents")
                    .foregroundStyle(.secondary)
            } else {
                Menu {
                    ForEach(agentOptionsIncludingCurrent) { agent in
                        Button {
                            selectLeader(agent.id)
                        } label: {
                            GaryxMenuSelectionLabel(
                                title: agent.displayName,
                                selected: normalizedLeader == agent.id,
                                fallbackSystemImage: "person"
                            )
                        }
                    }
                } label: {
                    GaryxFormMenuValueLabel(value: leaderLabel)
                }
            }
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
