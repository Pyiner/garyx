import SwiftUI

struct GaryxBotsContent: View {
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        let bots = model.configuredBotAccountSettings
        VStack(alignment: .leading, spacing: 10) {
            if bots.isEmpty {
                if model.isRemoteStatePending {
                    GaryxLoadingPanelView(title: "Loading bots...")
                } else {
                    GaryxEmptyPanelView(
                        icon: "bubble.left.and.bubble.right",
                        title: "No bots configured",
                        text: ""
                    )
                }
            } else {
                GaryxSectionBlock(title: "Bots") {
                    GaryxCompactListGroup {
                        ForEach(Array(bots.enumerated()), id: \.element.id) { index, bot in
                            GaryxConfiguredBotConfigRow(bot: bot)
                            if index < bots.count - 1 {
                                GaryxCompactRowDivider()
                            }
                        }
                    }
                }
            }
        }
        .task {
            if model.channelPlugins.isEmpty || model.gatewaySettingsDocument.isEmpty {
                await model.refreshRemoteState()
            }
        }
    }
}

struct GaryxConfiguredBotConfigRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let bot: GaryxConfiguredBotAccountSettings
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false

    var body: some View {
        GaryxRowActionMenu(actions: actions) {
            Button {
                showsEditForm = true
            } label: {
                HStack(alignment: .center, spacing: 10) {
                    GaryxChannelLogoView(
                        channel: bot.channel,
                        label: bot.displayName,
                        iconDataUrl: iconDataUrl,
                        diameter: 28
                    )

                    VStack(alignment: .leading, spacing: 3) {
                        Text(bot.displayName)
                            .font(GaryxFont.subheadline(weight: .semibold))
                            .foregroundStyle(.primary)
                            .garyxReadingLineLimit()
                        Text(detailLine)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .garyxReadingLineLimit()
                            .truncationMode(.middle)
                    }

                    Spacer(minLength: 6)

                    GaryxStatusPill(text: bot.enabled ? "Enabled" : "Paused", tone: bot.enabled ? .good : .muted)
                }
                .padding(.horizontal, 9)
                .padding(.vertical, 8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(GaryxPressableRowStyle())
            .accessibilityLabel("Open \(bot.displayName)")
            .accessibilityHint("Shows bot account details.")
        }
        .garyxFullScreenCover(isPresented: $showsEditForm) {
            GaryxBotAccountForm(account: bot)
        }
        .garyxConfirmationDialog("Delete bot account?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteConfiguredBotAccount(bot) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the channel account from the gateway configuration.")
        }
    }

    private var actions: [GaryxRowAction] {
        [
            GaryxRowAction(
                title: bot.enabled ? "Disable" : "Enable",
                systemImage: bot.enabled ? "pause.fill" : "play.fill",
                tone: .accent
            ) {
                Task { await model.setConfiguredBotAccountEnabled(bot, enabled: !bot.enabled) }
            },
            GaryxRowAction(title: "Edit", systemImage: "pencil") {
                showsEditForm = true
            },
            GaryxRowAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                showsDeleteConfirmation = true
            },
        ]
    }

    private var detailLine: String {
        let workspace = bot.workspaceDir?.garyxLastPathComponent.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let agent = bot.agentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let base = "\(channelDisplayName) Bot · \(bot.accountId)"
        if !workspace.isEmpty {
            return "\(base) · \(workspace)"
        }
        if !agent.isEmpty {
            return "\(base) · \(agent)"
        }
        return base
    }

    private var channelDisplayName: String {
        GaryxChannelIdentityPresentation.displayName(
            for: bot.channel,
            catalogDisplayName: GaryxChannelIconResolver.displayName(for: bot.channel, plugins: model.channelPlugins)
        )
    }

    private var iconDataUrl: String? {
        GaryxChannelIconResolver.iconDataUrl(for: bot.channel, plugins: model.channelPlugins)
    }
}

struct GaryxBotAccountForm: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    let account: GaryxConfiguredBotAccountSettings?

    @State private var initialized = false
    @State private var channel = ""
    @State private var accountId = ""
    @State private var generatedAccountId = ""
    @State private var displayName = ""
    @State private var enabled = true
    @State private var agentId: String?
    @State private var workspaceDir = ""
    @State private var workspaceMode = "local"
    @State private var configValues: [String: GaryxJSONValue] = [:]
    @State private var errorText: String?
    @State private var showsAgentPicker = false
    @State private var hasChosenAgent = false

    private var isEditing: Bool { account != nil }

    var body: some View {
        GaryxFormSheet(
            title: isEditing ? "Edit Bot" : "Add Bot",
            canSave: canSubmit && !model.isSavingBotSettings,
            onSave: { Task { await save() } }
        ) {
            formContent
        }
        .onAppear(perform: initializeIfNeeded)
        .task {
            await model.refreshAgentTargetsIfNeeded()
            applyPreferredAgentIfNeeded()
        }
        .onChange(of: model.effectiveDefaultAgentId) { _, _ in
            applyPreferredAgentIfNeeded()
        }
        .onChange(of: model.agentTargets) { _, _ in
            applyPreferredAgentIfNeeded()
        }
        .onChange(of: channel) { _, _ in
            guard initialized, !isEditing else { return }
            resetGeneratedAccountIdIfNeeded()
            applySchemaDefaults(replacing: true)
        }
    }

    private var formContent: some View {
        Group {
            GaryxFormGroupedSection(title: "Account") {
                channelPicker
                GaryxFormTextFieldRow(
                    title: "Account ID",
                    text: $accountId,
                    valuePlacement: .below,
                    autocapitalization: .never,
                    autocorrectionDisabled: true,
                    isReadOnly: isEditing
                )
                GaryxFormTextFieldRow(
                    title: "Display name",
                    text: $displayName,
                    placeholder: "Optional"
                )
                agentPicker
                GaryxWorkspacePathSelectionRow(
                    title: "Working directory",
                    path: $workspaceDir,
                    placeholder: "Optional",
                    allowsEmpty: true
                )
                Picker("Workspace mode", selection: $workspaceMode) {
                    Text("Local").tag("local")
                    Text("Worktree").tag("worktree")
                }
                .pickerStyle(.segmented)
                // UISegmentedControl owns a fixed single-line track; cap its
                // labels at XXL while every surrounding form row is unbounded.
                .garyxTypographyBoundary(.segmentedControlChrome)
                GaryxFormRow(title: "Enabled") {
                    Toggle("Enabled", isOn: $enabled)
                        .labelsHidden()
                }
            }

            GaryxFormGroupedSection(title: "Channel Auth") {
                if let selectedPlugin {
                    let fields = schemaFields(for: selectedPlugin)
                    ForEach(fields) { field in
                        GaryxBotConfigFieldEditor(
                            field: field,
                            value: binding(for: field)
                        )
                    }
                    if fields.isEmpty {
                        Text("This channel does not declare any manual configuration fields.")
                            .font(GaryxFont.callout())
                            .foregroundStyle(.secondary)
                    }
                } else {
                    Text("Channel catalog is still loading.")
                        .font(GaryxFont.callout())
                        .foregroundStyle(.secondary)
                }
            }

            if let errorText {
                GaryxFormErrorText(text: errorText)
            }
        }
    }

    private var availablePlugins: [GaryxChannelPluginCatalogEntry] {
        model.channelPlugins
            .filter { !$0.id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && $0.id != "api" }
            .sorted { lhs, rhs in
                lhs.displayName.localizedCaseInsensitiveCompare(rhs.displayName) == .orderedAscending
            }
    }

    private var selectedPlugin: GaryxChannelPluginCatalogEntry? {
        availablePlugins.first { $0.id.caseInsensitiveCompare(channel) == .orderedSame }
    }

    private var selectedChannelDisplayName: String {
        guard let plugin = selectedPlugin else { return channel }
        return plugin.displayName.isEmpty ? plugin.id : plugin.displayName
    }

    private var channelPicker: some View {
        GaryxFormMenuRow(title: "Channel", value: selectedChannelDisplayName) {
            Picker("Channel", selection: $channel) {
                ForEach(availablePlugins) { plugin in
                    Text(plugin.displayName.isEmpty ? plugin.id : plugin.displayName)
                        .tag(plugin.id)
                }
            }
            .labelsHidden()
            .pickerStyle(.inline)
        }
        .disabled(isEditing || availablePlugins.isEmpty)
    }

    private var agentPicker: some View {
        GaryxFormRow(
            title: "Agent",
            onTap: { showsAgentPicker = true }
        ) {
            GaryxBotAgentPickerControl(
                configuredAgentId: agentSelection,
                isPresented: $showsAgentPicker
            )
        }
    }

    private var canSubmit: Bool {
        !channel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !accountId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func initializeIfNeeded() {
        guard !initialized else { return }
        initialized = true
        if let account {
            channel = account.channel
            accountId = account.accountId
            displayName = account.displayName == account.accountId ? "" : account.displayName
            enabled = account.enabled
            agentId = account.agentId?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty
            workspaceDir = account.workspaceDir ?? ""
            workspaceMode = account.workspaceMode == "worktree" ? "worktree" : "local"
            configValues = account.config
        } else {
            channel = availablePlugins.first?.id ?? channel
            enabled = true
            agentId = preferredAgentId
            workspaceMode = "local"
            resetGeneratedAccountIdIfNeeded()
            applySchemaDefaults(replacing: false)
        }
    }

    private var preferredAgentId: String? {
        GaryxBotAgentPickerPresentation.preferredConfiguredAgentId(
            targets: model.agentTargets,
            effectiveDefaultAgentId: model.effectiveDefaultAgentId
        )
    }

    private var agentSelection: Binding<String?> {
        Binding {
            agentId
        } set: { next in
            hasChosenAgent = true
            agentId = next
        }
    }

    private func applyPreferredAgentIfNeeded() {
        guard initialized, !isEditing, !hasChosenAgent, agentId == nil else { return }
        agentId = preferredAgentId
    }

    private func resetGeneratedAccountIdIfNeeded() {
        guard !channel.isEmpty else { return }
        if accountId.isEmpty || accountId == generatedAccountId {
            let next = defaultAccountId(for: channel)
            generatedAccountId = next
            accountId = next
        }
    }

    private func defaultAccountId(for channel: String) -> String {
        GaryxBotAccountIdDefaults.defaultAccountId(
            channel: channel,
            existingAccountIds: Set(model.configuredBotAccountSettings.map(\.accountId))
        )
    }

    private func applySchemaDefaults(replacing: Bool) {
        guard let selectedPlugin else { return }
        configValues = GaryxBotConfigValues.applyingSchemaDefaults(
            to: configValues,
            fields: schemaFields(for: selectedPlugin),
            replacing: replacing
        )
    }

    private func binding(for field: GaryxBotSchemaField) -> Binding<GaryxJSONValue> {
        Binding(
            get: {
                GaryxBotConfigValues.editorValue(for: field, config: configValues)
            },
            set: { next in
                configValues[field.key] = next
            }
        )
    }

    private func save() async {
        errorText = nil
        let trimmedChannel = channel.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedAccountId = accountId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedChannel.isEmpty else {
            errorText = "Channel is required"
            return
        }
        guard !trimmedAccountId.isEmpty else {
            errorText = "Account ID is required"
            return
        }
        let fields = selectedPlugin.map { schemaFields(for: $0) } ?? []
        let normalizedConfig = GaryxBotConfigValues.normalized(config: configValues, fields: fields)
        let input = GaryxConfiguredBotAccountInput(
            channel: trimmedChannel,
            accountId: trimmedAccountId,
            displayName: displayName,
            enabled: enabled,
            agentId: agentId?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty,
            workspaceDir: workspaceDir.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty,
            workspaceMode: workspaceMode,
            config: normalizedConfig
        )
        if await model.saveConfiguredBotAccount(input, original: account) {
            dismiss()
        } else {
            errorText = model.lastError ?? "Save failed"
        }
    }

    private func schemaFields(for plugin: GaryxChannelPluginCatalogEntry) -> [GaryxBotSchemaField] {
        GaryxBotSchemaField.fields(from: plugin.schema)
    }
}

private struct GaryxBotConfigFieldEditor: View {
    let field: GaryxBotSchemaField
    @Binding var value: GaryxJSONValue
    @FocusState private var isFocused: Bool

    var body: some View {
        if field.kind == .boolean {
            GaryxFormRow(title: field.label) {
                Toggle(field.label, isOn: Binding(
                    get: { GaryxBotConfigValues.boolValue(value) ?? false },
                    set: { value = .bool($0) }
                ))
                .labelsHidden()
            }
        } else if !field.enumValues.isEmpty {
            GaryxFormMenuRow(title: field.label, value: GaryxBotConfigValues.stringValue(value)) {
                Picker(field.label, selection: Binding(
                    get: { GaryxBotConfigValues.stringValue(value) },
                    set: { value = .string($0) }
                )) {
                    ForEach(field.enumValues, id: \.self) { option in
                        Text(option).tag(option)
                    }
                }
                .labelsHidden()
                .pickerStyle(.inline)
            }
        } else {
            textEntry
        }
    }

    private var textEntry: some View {
        VStack(alignment: .leading, spacing: 0) {
            GaryxFormRow(
                title: field.label,
                required: field.required,
                valuePlacement: .below
            ) {
                editor
            }

            if let description = field.description, !description.isEmpty {
                Text(description)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.leading)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 16)
                    .padding(.bottom, 12)
            }
        }
    }

    @ViewBuilder
    private var editor: some View {
        if field.secret {
            SecureField(field.placeholder, text: Binding(
                get: { GaryxBotConfigValues.stringValue(value) },
                set: { value = GaryxBotConfigValues.fieldValue(fromEditorText: $0, kind: field.kind) }
            ))
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .font(GaryxFont.callout())
            .garyxReadingLineLimit()
            .focused($isFocused)
            .accessibilityLabel(field.label)
            .textFieldStyle(.plain)
        } else {
            TextField(field.placeholder, text: Binding(
                get: { GaryxBotConfigValues.stringValue(value) },
                set: { value = GaryxBotConfigValues.fieldValue(fromEditorText: $0, kind: field.kind) }
            ))
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .keyboardType(field.kind == .number ? .decimalPad : .default)
            .font(GaryxFont.callout())
            .garyxReadingLineLimit()
            .focused($isFocused)
            .accessibilityLabel(field.label)
            .textFieldStyle(.plain)
        }
    }
}

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }
}
