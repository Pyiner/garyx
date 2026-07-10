import Foundation
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct GaryxAgentsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateAgent = false

    var body: some View {
        GaryxPanelScaffold(
            title: "Agents",
            subtitle: "\(model.agents.count) agents",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            VStack(alignment: .leading, spacing: 18) {
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
            }
        } actions: {
            GaryxAddToolbarButton(label: "New Agent") {
                showsCreateAgent = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateAgent) {
            GaryxCreateAgentCard()
        }
        .fullScreenCover(item: $model.selectedAgentDetail) { agent in
            GaryxFormSheet(title: "Agent Detail") {
                GaryxAgentDetailCard(agent: agent)
            }
        }
    }
}

struct GaryxAgentDetailCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let agent: GaryxAgentSummary
    @State private var showsEditForm = false

    var body: some View {
        Group {
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
                env: .constant(.empty),
                builtIn: displayAgent.builtIn,
                workspacePaths: model.userWorkspacePaths
            )
            .fullScreenCover(isPresented: $showsEditForm) {
                GaryxAgentEditSheet(agent: displayAgent) { updatedAgent in
                    model.selectedAgentDetail = updatedAgent
                }
            }

            if !displayAgent.builtIn {
                Section {
                    Button {
                        showsEditForm = true
                    } label: {
                        Label("Edit Agent", systemImage: "pencil")
                            .fontWeight(.semibold)
                            .frame(maxWidth: .infinity)
                    }
                }
            }
        }
    }

    private var displayAgent: GaryxAgentSummary {
        model.agents.first(where: { $0.id == agent.id }) ?? agent
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
                label: avatarLabel,
                providerType: providerType,
                builtIn: builtIn,
                diameter: 76
            )
            .accessibilityLabel("Agent avatar preview")
            .padding(14)
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
        return trimmedIdentifier.isEmpty ? "Agent" : trimmedIdentifier
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
    @Binding var env: GaryxAgentEnvDraft
    var builtIn = false
    let workspacePaths: [String]
    var onGenerate: ((String) async -> String?)?
    var onError: ((String) -> Void)?

    var body: some View {
        Group {
            if mode.isEditable, let onGenerate, let onError {
                GaryxAvatarEditorSection(
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
                        valuePlacement: .below,
                        autocapitalization: .never,
                        autocorrectionDisabled: true
                    )
                    GaryxFormTextFieldRow(
                        title: "Display name",
                        text: $displayName,
                        placeholder: "Optional"
                    )
                } else {
                    GaryxAgentReadOnlyTextRow(title: "Agent ID", value: agentId)
                    GaryxAgentReadOnlyTextRow(title: "Display name", value: displayName)
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
                    GaryxFormReadOnlyRow(title: "Provider", value: GaryxAgentProviderPickerPresentation.label(for: providerType))
                    GaryxFormReadOnlyRow(title: "Model", value: modelDisplayValue)
                    if !modelReasoningEffort.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
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
                    GaryxFormTextAreaRow(
                        title: "System Prompt",
                        text: $systemPrompt,
                        minHeight: 132,
                        lineLimits: 2...6,
                        offersFocusedEditor: true
                    )
                } else {
                    GaryxFormReadOnlyMultilineRow(
                        title: "Default workspace",
                        value: workspace,
                        placeholder: "None",
                        minHeight: 44,
                        valuePlacement: .below
                    )
                    GaryxFormReadOnlyMultilineRow(
                        title: "System Prompt",
                        value: systemPrompt,
                        placeholder: "Provider default",
                        minHeight: 132,
                        valuePlacement: .below
                    )
                }
            }

            if mode.isEditable {
                GaryxAgentEnvEditorSection(draft: $env)
            }
        }
    }

    private var modelDisplayValue: String {
        let trimmed = modelName.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "Provider default" : modelName
    }
}

private struct GaryxAgentEnvEditorSection: View {
    @Binding var draft: GaryxAgentEnvDraft
    @State private var viewMode: EnvViewMode = .form
    @State private var envText: String = ""
    @FocusState private var isTextEditorFocused: Bool

    private enum EnvViewMode: String, CaseIterable {
        case form = "Form"
        case text = "Text"
    }

    var body: some View {
        Section {
            Picker("View", selection: viewModeBinding) {
                ForEach(EnvViewMode.allCases, id: \.self) { mode in
                    Text(mode.rawValue).tag(mode)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()

            if viewMode == .text {
                TextEditor(text: envTextBinding)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled(true)
                    .font(.system(.footnote, design: .monospaced))
                    .scrollContentBackground(.hidden)
                    .focused($isTextEditorFocused)
                    .accessibilityLabel("Environment variables")
                    .frame(minHeight: 160, alignment: .topLeading)
            } else {
                if draft.rows.isEmpty {
                    Text("No environment variables")
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(draft.rows) { row in
                        envRow(row)
                    }
                }

                Button {
                    draft.addRow()
                } label: {
                    Label("Add Variable", systemImage: "plus")
                }
            }
        } header: {
            Text("Environment Variables")
                .textCase(nil)
        } footer: {
            Text(envHint)
        }
    }

    private var envHint: String {
        if viewMode == .text {
            return "One KEY=value per line. Values are passed verbatim—no quoting is added, numbers stay plain. Lines starting with # are ignored."
        }
        return "Environment variables are passed to this agent’s provider runs. They may appear in command output or logs—avoid secrets you can’t rotate."
    }

    private var viewModeBinding: Binding<EnvViewMode> {
        Binding(
            get: { viewMode },
            set: { next in
                if next == .text, viewMode != .text {
                    envText = draft.envText()
                }
                viewMode = next
            },
        )
    }

    private var envTextBinding: Binding<String> {
        Binding(
            get: { envText },
            set: { next in
                envText = next
                draft.applyEnvText(next)
            },
        )
    }

    private func envRow(_ row: GaryxAgentEnvRow) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("Variable")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.secondary)
                Spacer(minLength: 0)
                Button(role: .destructive) {
                    draft.removeRow(id: row.id)
                } label: {
                    Image(systemName: "trash")
                }
                .accessibilityLabel("Remove variable")
            }

            LabeledContent("Name") {
                TextField("KEY", text: keyBinding(row))
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled(true)
                    .multilineTextAlignment(.trailing)
                    .textFieldStyle(.plain)
            }
            LabeledContent("Value") {
                TextField("value", text: valueBinding(row))
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled(true)
                    .multilineTextAlignment(.trailing)
                    .textFieldStyle(.plain)
            }
        }
        .padding(.vertical, 2)
    }

    private func keyBinding(_ row: GaryxAgentEnvRow) -> Binding<String> {
        Binding(get: { row.key }, set: { draft.updateKey(id: row.id, $0) })
    }

    private func valueBinding(_ row: GaryxAgentEnvRow) -> Binding<String> {
        Binding(get: { row.value }, set: { draft.updateValue(id: row.id, $0) })
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
    @State private var envDraft = GaryxAgentEnvDraft.empty

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
                env: $envDraft,
                workspacePaths: model.userWorkspacePaths
            ) { stylePrompt in
                await model.generateAvatar(
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
            && !envDraft.hasInvalidKey
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
            systemPrompt: systemPrompt,
            env: envDraft.currentEnvMap()
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
    @State private var envDraft = GaryxAgentEnvDraft.empty

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
                env: $envDraft,
                builtIn: agent.builtIn,
                workspacePaths: model.userWorkspacePaths
            ) { stylePrompt in
                await model.generateAvatar(
                    identifier: agentId,
                    displayName: displayName,
                    stylePrompt: stylePrompt
                )
            } onError: { message in
                model.lastError = message
            }
        }
        .onAppear {
            fillDraft()
            Task { await seedAuthoritativeEnv() }
        }
    }

    private var canSaveAgent: Bool {
        !agentId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !providerType.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !envDraft.hasInvalidKey
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
        envDraft = .seeded(from: agent.providerEnv)
    }

    // Re-seed env from the authoritative agent (a restored cache snapshot strips
    // provider_env). `reseedIfPristine` leaves in-progress user edits untouched.
    private func seedAuthoritativeEnv() async {
        let env = await model.authoritativeProviderEnv(for: agent)
        envDraft.reseedIfPristine(from: env)
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
            systemPrompt: systemPrompt,
            envIntent: envDraft.resolvedIntent()
        ) else { return }
        dismiss()
        onSaved?(updated)
    }
}


private struct GaryxAvatarEditorSection: View {
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
            VStack(alignment: .center, spacing: 12) {
                GaryxAgentAvatarView(
                    agentId: trimmedIdentifier,
                    avatarDataUrl: avatarDataUrl,
                    label: avatarLabel,
                    providerType: providerType,
                    builtIn: builtIn,
                    diameter: 76
                )
                .accessibilityLabel("Agent avatar preview")

                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 10) {
                        avatarActions
                    }
                    VStack(spacing: 10) {
                        avatarActions
                    }
                }
            }
            .padding(14)
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
        .tint(Color.primary)
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

    private var trimmedIdentifier: String {
        identifier.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var avatarLabel: String {
        let name = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        if !name.isEmpty {
            return name
        }
        return trimmedIdentifier.isEmpty ? "Agent" : trimmedIdentifier
    }

    private var canGenerate: Bool {
        !trimmedIdentifier.isEmpty || !displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var generationFingerprint: String {
        [
            "agent",
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
            "agent",
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
        await GaryxDataURLImageCache.predecodeAgentAvatar(from: generated)
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
            await GaryxDataURLImageCache.predecodeAgentAvatar(from: prepared)
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
                .font(Font.footnote.weight(.semibold))
                .lineLimit(1)
        }
        .foregroundStyle(.primary)
        .padding(.horizontal, 12)
        .frame(maxWidth: .infinity)
        .frame(height: 38)
        .background(Color.primary.opacity(0.055), in: Capsule())
        .overlay {
            Capsule()
                .stroke(Color.primary.opacity(0.08), lineWidth: 1)
        }
        .frame(minHeight: 44)
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
        GaryxAgentProviderPickerPresentation.label(for: normalizedProvider)
    }

    private var providerOptionsIncludingCurrent: [GaryxAgentProviderPickerOption] {
        GaryxAgentProviderPickerPresentation.options(includingCurrent: providerType)
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
    let options: [GaryxAgentProviderPickerOption]
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
                .garyxVerticalScrollContentWidth()
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
