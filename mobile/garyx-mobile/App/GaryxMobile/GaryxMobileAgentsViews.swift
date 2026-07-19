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
        .garyxFullScreenCover(isPresented: $showsCreateAgent) {
            GaryxCreateAgentCard()
        }
        .garyxFullScreenCover(item: $model.selectedAgentDetail) { agent in
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
                agentId: displayAgent.id,
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
            .garyxFullScreenCover(isPresented: $showsEditForm) {
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
    case create
    case edit
    case readOnly

    var isEditable: Bool {
        self != .readOnly
    }

    var identityFooter: String? {
        switch self {
        case .create:
            return "Agent ID is generated from Name."
        case .edit:
            return "Agent ID can’t be changed after creation."
        case .readOnly:
            return nil
        }
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
    let agentId: String
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
    var nameValidationMessage: String? = nil
    var environmentValidationMessage: String? = nil
    var nameFocusToken = 0
    var onGenerate: ((String) async -> GaryxAvatarGenerationOutcome)?
    var onError: ((String) -> Void)?
    @FocusState private var nameIsFocused: Bool

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
                    onError: onError,
                    onNameValidationFailed: {
                        nameIsFocused = true
                    }
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

            Section {
                if mode.isEditable {
                    LabeledContent {
                        TextField("", text: $displayName)
                            .multilineTextAlignment(.trailing)
                            .textFieldStyle(.plain)
                            .focused($nameIsFocused)
                            .accessibilityLabel("Name, required")
                    } label: {
                        HStack(spacing: 4) {
                            Text("Name")
                            Text("*")
                                .fontWeight(.semibold)
                                .foregroundStyle(GaryxTheme.danger)
                        }
                    }
                    LabeledContent("Agent ID") {
                        Text(agentId.isEmpty ? "Not available" : agentId)
                            .font(.system(.body, design: .monospaced))
                            .foregroundStyle(agentId.isEmpty ? .tertiary : .secondary)
                            .multilineTextAlignment(.trailing)
                            .textSelection(.enabled)
                            .accessibilityLabel(agentId.isEmpty ? "Agent ID unavailable" : "Agent ID \(agentId)")
                    }
                } else {
                    GaryxAgentReadOnlyTextRow(title: "Agent ID", value: agentId)
                    GaryxAgentReadOnlyTextRow(title: "Name", value: displayName)
                    GaryxAgentReadOnlyTextRow(title: "Type", value: builtIn ? "Built-in" : "Custom")
                }
            } header: {
                Text("Identity")
                    .textCase(nil)
            } footer: {
                if mode.isEditable {
                    VStack(alignment: .leading, spacing: 4) {
                        if let nameValidationMessage {
                            Text(nameValidationMessage)
                                .foregroundStyle(GaryxTheme.danger)
                                .accessibilityIdentifier("agent-name-error")
                        }
                        if let footer = mode.identityFooter {
                            Text(footer)
                        }
                    }
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
                GaryxAgentEnvEditorSection(
                    draft: $env,
                    validationMessage: environmentValidationMessage
                )
            }
        }
        .onChange(of: nameFocusToken) { _, _ in
            nameIsFocused = true
        }
    }

    private var modelDisplayValue: String {
        let trimmed = modelName.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "Provider default" : modelName
    }
}

private struct GaryxAgentEnvEditorSection: View {
    @Binding var draft: GaryxAgentEnvDraft
    var validationMessage: String?
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
            VStack(alignment: .leading, spacing: 4) {
                if let validationMessage {
                    Text(validationMessage)
                        .foregroundStyle(GaryxTheme.danger)
                }
                Text(envHint)
            }
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
    @State private var draft = GaryxCustomAgentDraft.create()
    @State private var isSaving = false
    @State private var submissionError: String?
    @State private var nameFocusToken = 0

    var body: some View {
        GaryxFormSheet(
            title: "New Agent",
            canSave: draft.canSubmit && !isSaving,
            saveTitle: "Create",
            isSaving: isSaving,
            onSave: { Task { await createAgent() } }
        ) {
            GaryxAgentFormContent(
                mode: .create,
                agentId: draft.agentId,
                displayName: $draft.displayName,
                providerType: $draft.providerType,
                modelName: $draft.model,
                modelReasoningEffort: $draft.modelReasoningEffort,
                workspace: $draft.defaultWorkspaceDir,
                avatarDataUrl: avatarBinding,
                systemPrompt: $draft.systemPrompt,
                env: $draft.env,
                workspacePaths: model.userWorkspacePaths,
                nameValidationMessage: draft.nameValidationMessage,
                environmentValidationMessage: draft.environmentValidationMessage,
                nameFocusToken: nameFocusToken
            ) { stylePrompt in
                await model.generateAvatar(
                    identifier: draft.agentId,
                    displayName: draft.displayName,
                    stylePrompt: stylePrompt
                )
            } onError: { message in
                model.lastError = message
            }
            if let submissionError {
                Section {
                    Text(submissionError)
                        .foregroundStyle(GaryxTheme.danger)
                        .accessibilityIdentifier("agent-create-error")
                }
            }
        }
    }

    private var avatarBinding: Binding<String> {
        Binding(
            get: { draft.avatarDataUrl },
            set: { draft.setAvatarDataUrl($0) }
        )
    }

    private func createAgent() async {
        guard !isSaving, let request = draft.makeRequest(), draft.createCollision == nil else { return }
        isSaving = true
        submissionError = nil
        defer { isSaving = false }

        switch await model.createAgent(request) {
        case .saved:
            dismiss()
        case .failed(.createConflict):
            draft.recordCreateConflict()
            nameFocusToken += 1
        case .failed(.other(let message)):
            submissionError = message
        case .failed:
            submissionError = "Couldn’t create this agent. Try again."
        case .superseded:
            break
        }
    }
}


private struct GaryxAgentEditSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let agent: GaryxAgentSummary
    var onSaved: ((GaryxAgentSummary) -> Void)?
    @State private var draft: GaryxCustomAgentDraft
    @State private var status = GaryxCustomAgentEditStatus.loading
    @State private var isSaving = false
    @State private var submissionError: String?

    init(
        agent: GaryxAgentSummary,
        onSaved: ((GaryxAgentSummary) -> Void)? = nil
    ) {
        self.agent = agent
        self.onSaved = onSaved
        _draft = State(initialValue: .edit(authoritative: agent))
    }

    var body: some View {
        GaryxFormSheet(
            title: "Edit Agent",
            canSave: canSaveAgent && !isSaving,
            isSaving: isSaving,
            onSave: { Task { await saveAgent() } }
        ) {
            editStatusSection
            GaryxAgentFormContent(
                mode: .edit,
                agentId: draft.agentId,
                displayName: $draft.displayName,
                providerType: $draft.providerType,
                modelName: $draft.model,
                modelReasoningEffort: $draft.modelReasoningEffort,
                workspace: $draft.defaultWorkspaceDir,
                avatarDataUrl: avatarBinding,
                systemPrompt: $draft.systemPrompt,
                env: $draft.env,
                builtIn: agent.builtIn,
                workspacePaths: model.userWorkspacePaths,
                nameValidationMessage: draft.nameValidationMessage,
                environmentValidationMessage: draft.environmentValidationMessage
            ) { stylePrompt in
                await model.generateAvatar(
                    identifier: draft.agentId,
                    displayName: draft.displayName,
                    stylePrompt: stylePrompt
                )
            } onError: { message in
                model.lastError = message
            }
            .disabled(!isReady || isSaving)

            if let submissionError {
                Section {
                    Text(submissionError)
                        .foregroundStyle(GaryxTheme.danger)
                        .accessibilityIdentifier("agent-edit-error")
                }
            }
        }
        .task(id: agent.id) {
            await reloadLatest()
        }
    }

    private var canSaveAgent: Bool {
        isReady && draft.canSubmit
    }

    private var isReady: Bool {
        if case .ready = status { return true }
        return false
    }

    private var avatarBinding: Binding<String> {
        Binding(
            get: { draft.avatarDataUrl },
            set: { draft.setAvatarDataUrl($0) }
        )
    }

    @ViewBuilder
    private var editStatusSection: some View {
        switch status {
        case .loading:
            Section {
                HStack(spacing: 10) {
                    ProgressView()
                    Text("Loading latest agent…")
                        .foregroundStyle(.secondary)
                }
                .accessibilityElement(children: .combine)
            }
        case .conflict:
            Section {
                Text("This agent changed elsewhere. Reload the latest version before saving.")
                    .foregroundStyle(GaryxTheme.danger)
                Button("Reload latest") {
                    Task { await reloadLatest() }
                }
            }
        case .deleted:
            Section {
                Text("This agent was deleted and can’t be saved.")
                    .foregroundStyle(GaryxTheme.danger)
            }
        case .loadFailed(let message):
            Section {
                Text(message)
                    .foregroundStyle(GaryxTheme.danger)
                Button("Retry") {
                    Task { await reloadLatest() }
                }
            }
        case .ready:
            EmptyView()
        }
    }

    @MainActor
    private func reloadLatest() async {
        guard !isSaving else { return }
        status = .loading
        submissionError = nil
        switch await model.loadAuthoritativeAgent(agentId: agent.id) {
        case .loaded(let authoritative):
            draft = .edit(authoritative: authoritative)
            status = .ready
        case .deleted:
            status = .deleted
        case .failed(let message):
            status = .loadFailed(message: message)
        case .superseded:
            status = .loadFailed(message: "The active gateway changed. Reopen this editor and try again.")
        }
    }

    @MainActor
    private func saveAgent() async {
        guard canSaveAgent, !isSaving, let request = draft.makeRequest() else { return }
        isSaving = true
        submissionError = nil
        defer { isSaving = false }

        switch await model.updateAgent(agentId: draft.agentId, request: request) {
        case .saved(let updated):
            dismiss()
            onSaved?(updated)
        case .failed(.editConflict(let currentUpdatedAt)):
            status = .conflict(currentUpdatedAt: currentUpdatedAt)
        case .failed(.deleted):
            status = .deleted
        case .failed(.other(let message)):
            submissionError = message
        case .failed:
            submissionError = "Couldn’t save this agent. Try again."
        case .superseded:
            break
        }
    }
}


private struct GaryxAvatarEditorSection: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.garyxPrefersCrossFadeTransitions) private var prefersCrossFadeTransitions
    @EnvironmentObject private var model: GaryxMobileModel
    let identifier: String
    let displayName: String
    let providerType: String
    var builtIn = false
    @Binding var avatarDataUrl: String
    let onGenerate: (String) async -> GaryxAvatarGenerationOutcome
    let onError: (String) -> Void
    let onNameValidationFailed: () -> Void

    @State private var editorState = GaryxMobileAvatarEditorState()
    @State private var showsPhotoPicker = false
    @State private var selectedPhotoItem: PhotosPickerItem?
    @State private var showsStyleSheet = false
    @State private var uploadTask: Task<Void, Never>?
    @State private var activeUploadId: UUID?
    @State private var uploadError: String?

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
                .id(avatarSignature)
                .transition(.opacity)
                .animation(
                    animatesTransition ? .easeInOut(duration: 0.2) : nil,
                    value: avatarSignature
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

                if let uploadError {
                    Text(uploadError)
                        .font(.footnote)
                        .foregroundStyle(GaryxTheme.danger)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityIdentifier("agent-avatar-upload-error")
                }
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .center)
        }
        .garyxSheet(isPresented: $showsStyleSheet) {
            GaryxAvatarStyleSheet(
                state: $editorState,
                cancellationToken: model.gatewayRequestToken,
                identifier: trimmedIdentifier,
                displayName: displayName.trimmingCharacters(in: .whitespacesAndNewlines),
                providerType: providerType,
                builtIn: builtIn,
                onGenerate: onGenerate,
                onUse: useGeneratedAvatar,
                onError: onError
            )
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
            .interactiveDismissDisabled(editorState.isGenerating)
        }
        .garyxPhotosPicker(
            isPresented: $showsPhotoPicker,
            selection: $selectedPhotoItem,
            matching: .images
        )
        .onChange(of: selectedPhotoItem) { _, item in
            guard let item else { return }
            startUpload(item)
        }
        .onDisappear {
            cancelUpload()
        }
    }

    private var animatesTransition: Bool {
        GaryxAccessibilityTransitionPolicy.animatesTransition(
            reduceMotion: reduceMotion,
            prefersCrossFadeTransitions: prefersCrossFadeTransitions
        )
    }

    @ViewBuilder
    private var avatarActions: some View {
        Button {
            showsPhotoPicker = true
        } label: {
            if uploadTask == nil {
                Label("Upload", systemImage: "photo")
            } else {
                HStack(spacing: 7) {
                    ProgressView()
                        .controlSize(.small)
                    Text("Uploading…")
                }
            }
        }
        .buttonStyle(.bordered)
        .frame(minHeight: 44)
        .disabled(uploadTask != nil)
        .accessibilityLabel("Upload avatar")

        Button {
            if canGenerate {
                editorState.reset(currentAvatarDataUrl: avatarDataUrl)
                showsStyleSheet = true
            } else {
                onNameValidationFailed()
            }
        } label: {
            Label(
                avatarDataUrl.isEmpty ? "Generate avatar" : "Generate new",
                systemImage: "sparkles"
            )
        }
        .buttonStyle(.bordered)
        .frame(minHeight: 44)
        .disabled(uploadTask != nil)
        .accessibilityLabel("Generate avatar")

        if !avatarDataUrl.isEmpty {
            Button(role: .destructive) {
                uploadError = nil
                avatarDataUrl = ""
            } label: {
                Label("Remove avatar", systemImage: "trash")
            }
            .buttonStyle(.bordered)
            .frame(minHeight: 44)
            .disabled(uploadTask != nil)
            .accessibilityLabel("Remove avatar")
        }
    }

    private func startUpload(_ item: PhotosPickerItem) {
        guard uploadTask == nil else {
            selectedPhotoItem = nil
            return
        }
        uploadError = nil
        let requestId = UUID()
        activeUploadId = requestId
        let fingerprint = uploadFingerprint
        uploadTask = Task {
            await uploadAvatar(
                from: item,
                requestId: requestId,
                fingerprint: fingerprint
            )
        }
    }

    private func cancelUpload() {
        uploadTask?.cancel()
        uploadTask = nil
        activeUploadId = nil
        selectedPhotoItem = nil
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
        !displayName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !trimmedIdentifier.isEmpty
    }

    private var uploadFingerprint: String {
        [
            "agent",
            String(avatarDataUrl.count),
            String(avatarDataUrl.prefix(80)),
            String(avatarDataUrl.suffix(80)),
        ].joined(separator: "\u{1F}")
    }

    private var avatarSignature: String {
        [
            String(avatarDataUrl.count),
            String(avatarDataUrl.prefix(40)),
            String(avatarDataUrl.suffix(40)),
        ].joined(separator: ":")
    }

    @MainActor
    private func uploadAvatar(
        from item: PhotosPickerItem,
        requestId: UUID,
        fingerprint: String
    ) async {
        defer {
            finishUpload(requestId: requestId)
        }

        do {
            guard let data = try await item.loadTransferable(type: Data.self) else {
                presentUploadError(
                    "Failed to read avatar image.",
                    requestId: requestId,
                    fingerprint: fingerprint
                )
                return
            }
            let prepared = try await Task.detached(priority: .utility) {
                try GaryxMobileAvatarImageNormalizer.normalizedDataUrl(fromImageData: data)
            }.value
            guard canApplyUpload(requestId: requestId, fingerprint: fingerprint) else { return }
            await GaryxDataURLImageCache.predecodeAgentAvatar(from: prepared)
            guard canApplyUpload(requestId: requestId, fingerprint: fingerprint) else { return }
            avatarDataUrl = prepared
        } catch is CancellationError {
            return
        } catch let error as GaryxMobileAvatarImageNormalizer.NormalizationError {
            presentUploadError(
                error.localizedDescription,
                requestId: requestId,
                fingerprint: fingerprint
            )
        } catch {
            presentUploadError(
                error.localizedDescription,
                requestId: requestId,
                fingerprint: fingerprint
            )
        }
    }

    private func canApplyUpload(requestId: UUID, fingerprint: String) -> Bool {
        !Task.isCancelled
            && activeUploadId == requestId
            && uploadFingerprint == fingerprint
    }

    private func presentUploadError(
        _ message: String,
        requestId: UUID,
        fingerprint: String
    ) {
        if canApplyUpload(requestId: requestId, fingerprint: fingerprint) {
            uploadError = message
            onError(message)
        }
    }

    private func finishUpload(requestId: UUID) {
        if activeUploadId == requestId {
            activeUploadId = nil
            uploadTask = nil
            selectedPhotoItem = nil
        }
    }

    private func useGeneratedAvatar(_ dataUrl: String) {
        uploadError = nil
        avatarDataUrl = dataUrl
    }
}

private struct GaryxAvatarStyleSheet: View {
    @Environment(\.dismiss) private var dismiss
    @Binding var state: GaryxMobileAvatarEditorState
    let cancellationToken: GaryxGatewayRequestToken
    let identifier: String
    let displayName: String
    let providerType: String
    let builtIn: Bool
    let onGenerate: (String) async -> GaryxAvatarGenerationOutcome
    let onUse: (String) -> Void
    let onError: (String) -> Void
    @State private var generationTask: Task<Void, Never>?
    @State private var waitTask: Task<Void, Never>?
    @State private var generationEpoch = 0
    @State private var showsLongWaitMessage = false
    @State private var successFeedback = 0
    @State private var failureFeedback = 0
    @AccessibilityFocusState private var failureIsFocused: Bool
    @AccessibilityFocusState private var useAvatarIsFocused: Bool

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    previewComparison

                    switch state.phase {
                    case .choosing:
                        Text("Choose a style, then generate a preview. Your current avatar won’t change until you use the result.")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    case .generating:
                        VStack(alignment: .leading, spacing: 4) {
                            Text("Generating avatar…")
                                .fontWeight(.semibold)
                            if showsLongWaitMessage {
                                Text("This can take a little while.")
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .accessibilityElement(children: .combine)
                    case .candidate:
                        VStack(alignment: .leading, spacing: 8) {
                            Text("Avatar ready")
                                .fontWeight(.semibold)
                            Text("Use avatar updates this form draft. Save the agent to persist it.")
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                            Button("Generate again") {
                                startGeneration()
                            }
                        }
                    case .failed(let failure):
                        VStack(alignment: .leading, spacing: 10) {
                            Text(failure.message)
                                .foregroundStyle(GaryxTheme.danger)
                                .accessibilityFocused($failureIsFocused)
                                .accessibilityIdentifier("agent-avatar-generation-error")
                            Button("Change style") {
                                state.changeStyle()
                            }
                        }
                    }
                }

                if state.phase == .choosing || state.phase == .generating {
                    Section("Style") {
                        ForEach(GaryxAvatarStyleOption.builtIn) { style in
                            GaryxAvatarStyleRow(
                                title: style.label,
                                isSelected: state.selectedStyleId == style.id
                            ) {
                                state.selectedStyleId = style.id
                            }
                        }

                        Button {
                            state.selectedStyleId = "custom"
                        } label: {
                            HStack {
                                Text("Custom style")
                                Spacer()
                                if state.selectedStyleId == "custom" {
                                    GaryxSelectionCheckmark(size: 14)
                                }
                            }
                            .contentShape(Rectangle())
                        }

                        if state.selectedStyleId == "custom" {
                            VStack(alignment: .leading, spacing: 8) {
                                Text("Describe the style")
                                    .font(.subheadline.weight(.semibold))
                                TextEditor(text: $state.customStyle)
                                    .frame(minHeight: 100)
                                    .accessibilityLabel("Custom avatar style")
                                if state.customStyle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                                    Text("For example: polished paper-cut icon with emerald accents")
                                        .font(.footnote)
                                        .foregroundStyle(.tertiary)
                                }
                            }
                        }
                    }
                    .disabled(state.isGenerating)
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Avatar style")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(state.leadingAction == .cancelGeneration ? "Cancel generation" : "Cancel") {
                        cancelAndDismiss()
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    switch state.primaryAction {
                    case .generate:
                        Button("Generate") {
                            startGeneration()
                        }
                        .fontWeight(.semibold)
                        .disabled(!state.canGenerate)
                    case .disabled:
                        Button(action: {}) {
                            HStack(spacing: 6) {
                                ProgressView()
                                    .controlSize(.small)
                                Text("Generating…")
                                    .fontWeight(.semibold)
                            }
                        }
                        .disabled(true)
                        .accessibilityLabel("Generating avatar")
                    case .use:
                        Button("Use") {
                            acceptCandidate()
                        }
                        .fontWeight(.semibold)
                        .accessibilityFocused($useAvatarIsFocused)
                    case .retry:
                        Button("Retry") {
                            startGeneration()
                        }
                        .fontWeight(.semibold)
                    }
                }
            }
        }
        .tint(GaryxTheme.controlTint)
        .sensoryFeedback(.success, trigger: successFeedback)
        .sensoryFeedback(.error, trigger: failureFeedback)
        .onDisappear {
            cancelGeneration(announces: false)
        }
        .onChange(of: cancellationToken) { _, _ in
            cancelAndDismiss()
        }
    }

    @ViewBuilder
    private var previewComparison: some View {
        if let candidate = state.candidateAvatarDataUrl, !candidate.isEmpty {
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .top, spacing: 24) {
                    avatarPreview(title: "Current", dataUrl: state.currentAvatarDataUrl, loading: false)
                    avatarPreview(title: "New", dataUrl: candidate, loading: state.isGenerating)
                }
                .frame(maxWidth: .infinity)

                VStack(spacing: 18) {
                    avatarPreview(title: "Current", dataUrl: state.currentAvatarDataUrl, loading: false)
                    avatarPreview(title: "New", dataUrl: candidate, loading: state.isGenerating)
                }
                .frame(maxWidth: .infinity)
            }
        } else {
            avatarPreview(
                title: "Current",
                dataUrl: state.currentAvatarDataUrl,
                loading: state.isGenerating
            )
            .frame(maxWidth: .infinity)
        }
    }

    private func avatarPreview(
        title: String,
        dataUrl: String,
        loading: Bool
    ) -> some View {
        GaryxAvatarGenerationPreview(
            title: title,
            identifier: identifier,
            displayName: displayName,
            providerType: providerType,
            builtIn: builtIn,
            dataUrl: dataUrl,
            isLoading: loading
        )
    }

    @MainActor
    private func startGeneration() {
        guard state.canGenerate else { return }
        generationEpoch += 1
        let epoch = generationEpoch
        guard let requestId = state.beginGeneration() else { return }
        let stylePrompt = state.activeStylePrompt
        showsLongWaitMessage = false
        announce("Generating avatar")

        waitTask?.cancel()
        waitTask = Task {
            try? await Task.sleep(nanoseconds: 8_000_000_000)
            guard !Task.isCancelled,
                  epoch == generationEpoch,
                  state.requestId == requestId else { return }
            showsLongWaitMessage = true
        }

        generationTask = Task {
            var outcome = await onGenerate(stylePrompt)
            guard owns(requestId: requestId, epoch: epoch) else { return }
            if Task.isCancelled {
                outcome = .cancelled
            }
            if case .success(let dataUrl) = outcome {
                await GaryxDataURLImageCache.predecodeAgentAvatar(from: dataUrl)
                guard owns(requestId: requestId, epoch: epoch) else { return }
            }

            let applied = state.resolve(outcome, requestId: requestId)
            guard applied else { return }
            waitTask?.cancel()
            showsLongWaitMessage = false
            switch outcome {
            case .success:
                successFeedback += 1
                useAvatarIsFocused = true
                announce("Avatar ready")
            case .failure(let failure):
                failureFeedback += 1
                failureIsFocused = true
                onError(failure.message)
                announce("Couldn’t generate avatar")
            case .cancelled, .superseded:
                break
            }
            if generationEpoch == epoch {
                generationTask = nil
            }
        }
    }

    @MainActor
    private func cancelAndDismiss() {
        if state.isGenerating {
            cancelGeneration(announces: true)
        }
        dismiss()
    }

    @MainActor
    private func cancelGeneration(announces: Bool) {
        guard state.isGenerating else { return }
        generationEpoch += 1
        generationTask?.cancel()
        waitTask?.cancel()
        generationTask = nil
        waitTask = nil
        showsLongWaitMessage = false
        _ = state.cancelGeneration()
        if announces {
            announce("Generation cancelled")
        }
    }

    @MainActor
    private func acceptCandidate() {
        guard let candidate = state.acceptCandidate() else { return }
        onUse(candidate)
        announce("New avatar selected")
        dismiss()
    }

    private func owns(requestId: UUID, epoch: Int) -> Bool {
        !Task.isCancelled
            && generationEpoch == epoch
            && state.requestId == requestId
            && state.isGenerating
    }

    private func announce(_ message: String) {
        UIAccessibility.post(notification: .announcement, argument: message)
    }
}

private struct GaryxAvatarGenerationPreview: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.garyxPrefersCrossFadeTransitions) private var prefersCrossFadeTransitions
    let title: String
    let identifier: String
    let displayName: String
    let providerType: String
    let builtIn: Bool
    let dataUrl: String
    let isLoading: Bool

    var body: some View {
        VStack(spacing: 8) {
            Text(title)
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
            ZStack {
                GaryxAgentAvatarView(
                    agentId: identifier,
                    avatarDataUrl: dataUrl,
                    label: displayName.isEmpty ? identifier : displayName,
                    providerType: providerType,
                    builtIn: builtIn,
                    diameter: 104
                )
                .id(imageSignature)
                .transition(.opacity)

                if isLoading {
                    Circle()
                        .fill(overlayColor)
                        .overlay {
                            VStack(spacing: 5) {
                                ProgressView()
                                    .tint(overlayForeground)
                                Text("Generating…")
                                    .font(.caption2.weight(.semibold))
                                    .foregroundStyle(overlayForeground)
                            }
                        }
                        .overlay {
                            if reduceTransparency {
                                Circle()
                                    .stroke(Color.primary.opacity(0.22), lineWidth: 1)
                            }
                        }
                        .transition(.opacity)
                        .accessibilityHidden(true)
                }
            }
            .frame(width: 104, height: 104)
            .animation(
                animatesTransition ? .easeInOut(duration: 0.18) : nil,
                value: imageSignature
            )
            .animation(
                animatesTransition ? .easeInOut(duration: 0.16) : nil,
                value: isLoading
            )
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(isLoading ? "\(title) avatar, generating" : "\(title) avatar")
        }
        .frame(minWidth: 120)
    }

    private var overlayColor: Color {
        if reduceTransparency {
            return Color(uiColor: .secondarySystemBackground).opacity(0.98)
        }
        return Color.primary.opacity(0.32)
    }

    private var overlayForeground: Color {
        reduceTransparency ? Color.primary : Color(uiColor: .systemBackground)
    }

    private var animatesTransition: Bool {
        GaryxAccessibilityTransitionPolicy.animatesTransition(
            reduceMotion: reduceMotion,
            prefersCrossFadeTransitions: prefersCrossFadeTransitions
        )
    }

    private var imageSignature: String {
        [String(dataUrl.count), String(dataUrl.prefix(32)), String(dataUrl.suffix(32))]
            .joined(separator: ":")
    }
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
        GaryxSwipeActionRow(id: "agent:\(agent.id)", actions: availabilitySwipeActions) {
            GaryxRowActionMenu(actions: agentMenuActions) {
                Button {
                    model.selectedAgentDetail = agent
                } label: {
                    agentIdentityRow
                }
                .buttonStyle(.plain)
                .contentShape(Rectangle())
            }
        }
        .garyxFullScreenCover(isPresented: $showsEditForm) {
            GaryxAgentEditSheet(agent: agent)
        }
        .garyxConfirmationDialog("Delete agent?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteAgent(agent) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the custom agent configuration.")
        }
    }

    private var availabilitySwipeActions: [GaryxRowAction] {
        [
            GaryxRowAction(
                title: agent.enabled ? "Disable" : "Enable",
                systemImage: agent.enabled ? "pause.circle" : "play.circle",
                tone: agent.enabled ? .warning : .accent
            ) {
                Task { await model.setAgentEnabled(agent, enabled: !agent.enabled) }
            },
        ]
    }

    private var agentMenuActions: [GaryxRowAction] {
        var actions: [GaryxRowAction] = []
        if GaryxAgentAvailabilityPresentation.allowsNewBindingActions(
            enabled: agent.enabled,
            standalone: agent.standalone
        ) {
            actions.append(
                GaryxRowAction(title: "Chat", systemImage: "message", tone: .accent) {
                    model.openAgentChatDraft(agent.id)
                }
            )
            actions.append(
                GaryxRowAction(title: "Use", systemImage: "checkmark.circle") {
                    Task { await model.setDefaultAgent(agent) }
                }
            )
        }
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

    private var agentIdentityRow: some View {
        HStack(spacing: 12) {
            GaryxAgentAvatarView(
                agentId: agent.id,
                avatarDataUrl: agent.avatarDataUrl,
                label: agent.displayName,
                providerType: agent.providerType,
                builtIn: agent.builtIn
            )
            VStack(alignment: .leading, spacing: 3) {
                Text(agent.displayName.isEmpty ? agent.id : agent.displayName)
                    .font(GaryxFont.body(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                Text(agent.id)
                    .font(GaryxFont.caption(weight: .medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 8)
            VStack(alignment: .trailing, spacing: 5) {
                GaryxStatusPill(
                    text: GaryxAgentAvailabilityPresentation.statusLabel(enabled: agent.enabled),
                    tone: agent.enabled ? .good : .muted
                )
                if let badge = defaultBadge {
                    GaryxStatusPill(
                        text: badge.label,
                        tone: badge.isMuted ? .muted : .good
                    )
                }
            }
        }
        .padding(10)
        .contentShape(Rectangle())
    }

    private var defaultBadge: GaryxAgentDefaultBadgeState? {
        GaryxAgentAvailabilityPresentation.defaultBadge(
            agentId: agent.id,
            enabled: agent.enabled,
            defaultAgentId: model.gatewayDefaultAgentId,
            effectiveDefaultAgentId: model.effectiveDefaultAgentId
        )
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
        .garyxSheet(isPresented: $showsProviderSheet) {
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
                .garyxSheet(isPresented: $showsModelSheet) {
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
            .garyxSheet(isPresented: $showsEffortSheet) {
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
                GaryxGlassPanel(cornerRadius: 28, shadowOpacity: 0.045) {
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
