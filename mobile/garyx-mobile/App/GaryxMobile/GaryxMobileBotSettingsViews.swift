import SwiftUI

struct GaryxBotsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateBot = false

    var body: some View {
        GaryxPanelScaffold(
            title: "Bots",
            subtitle: "\(model.configuredBotAccountSettings.count) configured",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            GaryxBotsContent()
        } actions: {
            GaryxAddToolbarButton(label: "Add Bot") {
                showsCreateBot = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateBot) {
            GaryxFormSheet(title: "Add Bot") {
                GaryxBotAccountForm(account: nil)
            }
        }
    }
}

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
                                GaryxCompactGroupDivider()
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
        GaryxSwipeActionRow(actions: actions) {
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
                        .lineLimit(1)
                    Text(detailLine)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }

                Spacer(minLength: 6)

                GaryxStatusPill(text: bot.enabled ? "Enabled" : "Paused", tone: bot.enabled ? .good : .muted)
            }
            .padding(.horizontal, 9)
            .padding(.vertical, 8)
        }
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(title: "Edit Bot") {
                GaryxBotAccountForm(account: bot)
            }
        }
        .confirmationDialog("Delete bot account?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteConfiguredBotAccount(bot) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the channel account from the gateway configuration.")
        }
    }

    private var actions: [GaryxSwipeAction] {
        [
            GaryxSwipeAction(
                title: bot.enabled ? "Disable" : "Enable",
                systemImage: bot.enabled ? "pause.fill" : "play.fill",
                tone: .accent
            ) {
                Task { await model.setConfiguredBotAccountEnabled(bot, enabled: !bot.enabled) }
            },
            GaryxSwipeAction(title: "Edit", systemImage: "pencil") {
                showsEditForm = true
            },
            GaryxSwipeAction(title: "Delete", systemImage: "trash", tone: .destructive) {
                showsDeleteConfirmation = true
            },
        ]
    }

    private var detailLine: String {
        let workspace = bot.workspaceDir?.garyxLastPathComponent.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let agent = bot.agentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let base = "\(garyxConfiguredBotChannelDisplayName(bot.channel)) Bot · \(bot.accountId)"
        if !workspace.isEmpty {
            return "\(base) · \(workspace)"
        }
        if !agent.isEmpty {
            return "\(base) · \(agent)"
        }
        return base
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
    @State private var agentId = "claude"
    @State private var workspaceDir = ""
    @State private var workspaceMode = "local"
    @State private var configValues: [String: GaryxJSONValue] = [:]
    @State private var errorText: String?

    private var isEditing: Bool { account != nil }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            GaryxSectionBlock(title: "Account") {
                VStack(alignment: .leading, spacing: 12) {
                    channelPicker
                    TextField("Account ID", text: $accountId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .disabled(isEditing)
                        .garyxInputStyle()
                    TextField("Display name", text: $displayName)
                        .garyxInputStyle()
                    agentPicker
                    TextField("Working directory", text: $workspaceDir)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .garyxInputStyle()
                    Picker("Workspace mode", selection: $workspaceMode) {
                        Text("Local").tag("local")
                        Text("Worktree").tag("worktree")
                    }
                    .pickerStyle(.segmented)
                    Toggle("Enabled", isOn: $enabled)
                        .tint(.blue)
                }
            }

            GaryxSectionBlock(title: "Channel Auth") {
                VStack(alignment: .leading, spacing: 12) {
                    if let selectedPlugin {
                        ForEach(schemaFields(for: selectedPlugin)) { field in
                            GaryxBotConfigFieldEditor(
                                field: field,
                                value: binding(for: field)
                            )
                        }
                        if schemaFields(for: selectedPlugin).isEmpty {
                            Text("This channel does not declare any manual configuration fields.")
                                .font(GaryxFont.caption())
                                .foregroundStyle(.secondary)
                        }
                    } else {
                        Text("Channel catalog is still loading.")
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                    }
                }
            }

            if let errorText {
                Text(errorText)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.red)
            }

            Button {
                Task { await save() }
            } label: {
                Label(model.isSavingBotSettings ? "Saving" : (isEditing ? "Save Bot" : "Create Bot"), systemImage: "checkmark")
            }
            .buttonStyle(GaryxPrimaryCompactButtonStyle())
            .disabled(model.isSavingBotSettings)
        }
        .garyxCardStyle()
        .onAppear(perform: initializeIfNeeded)
        .task {
            await model.refreshAgentTargetsIfNeeded()
            applyDefaultAgentIfNeeded()
        }
        .onChange(of: model.agentTargets) { _, _ in
            applyDefaultAgentIfNeeded()
        }
        .onChange(of: channel) { _, _ in
            guard initialized, !isEditing else { return }
            resetGeneratedAccountIdIfNeeded()
            applySchemaDefaults(replacing: true)
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

    private var channelPicker: some View {
        Picker("Channel", selection: $channel) {
            ForEach(availablePlugins) { plugin in
                Text(plugin.displayName.isEmpty ? plugin.id : plugin.displayName)
                    .tag(plugin.id)
            }
        }
        .disabled(isEditing || availablePlugins.isEmpty)
        .pickerStyle(.menu)
        .tint(.secondary)
    }

    private var agentPicker: some View {
        Picker("Agent", selection: $agentId) {
            Text("Claude").tag("claude")
            ForEach(model.agentTargets) { target in
                Text(target.title).tag(target.id)
            }
        }
        .pickerStyle(.menu)
        .tint(.secondary)
    }

    private func initializeIfNeeded() {
        guard !initialized else { return }
        initialized = true
        if let account {
            channel = account.channel
            accountId = account.accountId
            displayName = account.displayName == account.accountId ? "" : account.displayName
            enabled = account.enabled
            agentId = account.agentId?.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty ?? "claude"
            workspaceDir = account.workspaceDir ?? ""
            workspaceMode = account.workspaceMode == "worktree" ? "worktree" : "local"
            configValues = account.config
        } else {
            channel = availablePlugins.first?.id ?? channel
            enabled = true
            agentId = model.agentTargets.first(where: { $0.id == "claude" })?.id
                ?? model.agentTargets.first?.id
                ?? "claude"
            workspaceMode = "local"
            resetGeneratedAccountIdIfNeeded()
            applySchemaDefaults(replacing: false)
        }
    }

    private func applyDefaultAgentIfNeeded() {
        guard initialized, !isEditing, !model.agentTargets.isEmpty else { return }
        if !model.agentTargets.contains(where: { $0.id == agentId }) {
            agentId = model.agentTargets.first(where: { $0.id == "claude" })?.id
                ?? model.agentTargets.first?.id
                ?? agentId
        }
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
        let slug = channel
            .lowercased()
            .map { $0.isLetter || $0.isNumber ? String($0) : "-" }
            .joined()
            .split(separator: "-")
            .joined(separator: "-")
        let base = "\(slug.isEmpty ? "bot" : slug)-main"
        let existing = Set(model.configuredBotAccountSettings.map(\.accountId))
        if !existing.contains(base) {
            return base
        }
        for index in 2...99 {
            let candidate = "\(base)-\(index)"
            if !existing.contains(candidate) {
                return candidate
            }
        }
        return "\(base)-new"
    }

    private func applySchemaDefaults(replacing: Bool) {
        guard let selectedPlugin else { return }
        var next = replacing ? [:] : configValues
        for field in schemaFields(for: selectedPlugin) {
            if next[field.key] == nil, let defaultValue = field.defaultValue {
                next[field.key] = defaultValue
            }
        }
        configValues = next
    }

    private func binding(for field: GaryxBotSchemaField) -> Binding<GaryxJSONValue> {
        Binding(
            get: {
                configValues[field.key] ?? field.defaultValue ?? (field.kind == .boolean ? .bool(false) : .string(""))
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
        let normalizedConfig = normalizedConfigValues(fields: fields)
        let input = GaryxConfiguredBotAccountInput(
            channel: trimmedChannel,
            accountId: trimmedAccountId,
            displayName: displayName,
            enabled: enabled,
            agentId: agentId.trimmingCharacters(in: .whitespacesAndNewlines).nilIfEmpty,
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

    private func normalizedConfigValues(fields: [GaryxBotSchemaField]) -> [String: GaryxJSONValue] {
        var next = configValues
        for field in fields {
            let value = configValues[field.key] ?? field.defaultValue ?? .string("")
            switch field.kind {
            case .boolean:
                next[field.key] = .bool(garyxBotBoolValue(value) ?? false)
            case .number:
                let text = garyxBotStringValue(value).trimmingCharacters(in: .whitespacesAndNewlines)
                if text.isEmpty, !field.required {
                    next.removeValue(forKey: field.key)
                } else {
                    next[field.key] = .number(Double(text) ?? 0)
                }
            case .string:
                let text = garyxBotStringValue(value).trimmingCharacters(in: .whitespacesAndNewlines)
                if text.isEmpty, !field.required {
                    next.removeValue(forKey: field.key)
                } else {
                    next[field.key] = .string(text)
                }
            }
        }
        return next
    }

    private func schemaFields(for plugin: GaryxChannelPluginCatalogEntry) -> [GaryxBotSchemaField] {
        GaryxBotSchemaField.fields(from: plugin.schema)
    }
}

private struct GaryxBotConfigFieldEditor: View {
    let field: GaryxBotSchemaField
    @Binding var value: GaryxJSONValue

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 4) {
                Text(field.label)
                    .font(GaryxFont.caption(weight: .medium))
                    .foregroundStyle(.secondary)
                if field.required {
                    Text("*")
                        .font(GaryxFont.caption(weight: .semibold))
                        .foregroundStyle(.red)
                }
            }
            editor
            if let description = field.description, !description.isEmpty {
                Text(description)
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    @ViewBuilder
    private var editor: some View {
        if field.kind == .boolean {
            Toggle(field.label, isOn: Binding(
                get: { garyxBotBoolValue(value) ?? false },
                set: { value = .bool($0) }
            ))
            .labelsHidden()
            .tint(.blue)
        } else if !field.enumValues.isEmpty {
            Picker(field.label, selection: Binding(
                get: { garyxBotStringValue(value) },
                set: { value = .string($0) }
            )) {
                ForEach(field.enumValues, id: \.self) { option in
                    Text(option).tag(option)
                }
            }
            .pickerStyle(.menu)
            .tint(.secondary)
        } else if field.secret {
            SecureField(field.placeholder, text: Binding(
                get: { garyxBotStringValue(value) },
                set: { value = field.kind == .number ? .number(Double($0) ?? 0) : .string($0) }
            ))
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .garyxInputStyle()
        } else {
            TextField(field.placeholder, text: Binding(
                get: { garyxBotStringValue(value) },
                set: { value = field.kind == .number ? .number(Double($0) ?? 0) : .string($0) }
            ))
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .keyboardType(field.kind == .number ? .decimalPad : .default)
            .garyxInputStyle()
        }
    }
}

private struct GaryxBotSchemaField: Identifiable, Equatable {
    enum Kind {
        case string
        case boolean
        case number
    }

    var id: String { key }
    var key: String
    var label: String
    var kind: Kind
    var required: Bool
    var secret: Bool
    var enumValues: [String]
    var defaultValue: GaryxJSONValue?
    var description: String?
    var placeholder: String

    static func fields(from schema: [String: GaryxJSONValue]) -> [GaryxBotSchemaField] {
        let properties = garyxBotObjectValue(schema["properties"]) ?? [:]
        let required = Set((garyxBotArrayValue(schema["required"]) ?? []).compactMap(garyxBotStringValueIfPresent))
        return properties
            .compactMap { key, rawValue -> GaryxBotSchemaField? in
                guard let object = garyxBotObjectValue(rawValue) else { return nil }
                let type = garyxBotStringValueIfPresent(object["type"]) ?? "string"
                let enumValues = (garyxBotArrayValue(object["enum"]) ?? []).compactMap(garyxBotStringValueIfPresent)
                let kind: Kind
                switch type {
                case "boolean":
                    kind = .boolean
                case "number", "integer":
                    kind = .number
                default:
                    kind = .string
                }
                let xGaryx = garyxBotObjectValue(object["x-garyx"]) ?? [:]
                let secret = garyxBotBoolValue(xGaryx["secret"]) ?? false
                return GaryxBotSchemaField(
                    key: key,
                    label: key
                        .replacingOccurrences(of: "_", with: " ")
                        .split(separator: " ")
                        .map { $0.prefix(1).uppercased() + $0.dropFirst() }
                        .joined(separator: " "),
                    kind: kind,
                    required: required.contains(key),
                    secret: secret,
                    enumValues: enumValues,
                    defaultValue: object["default"],
                    description: garyxBotStringValueIfPresent(object["description"]),
                    placeholder: garyxBotStringValueIfPresent(object["description"]) ?? key
                )
            }
            .sorted { lhs, rhs in
                if lhs.required != rhs.required {
                    return lhs.required
                }
                return lhs.key.localizedCaseInsensitiveCompare(rhs.key) == .orderedAscending
            }
    }
}

private func garyxConfiguredBotChannelDisplayName(_ channel: String) -> String {
    let normalized = channel.trimmingCharacters(in: .whitespacesAndNewlines)
    switch normalized.lowercased() {
    case "telegram":
        return "Telegram"
    case "feishu":
        return "Feishu"
    case "weixin":
        return "Weixin"
    case "discord":
        return "Discord"
    case "api":
        return "API"
    default:
        return normalized.isEmpty ? "Channel" : normalized
    }
}

private func garyxBotObjectValue(_ value: GaryxJSONValue?) -> [String: GaryxJSONValue]? {
    guard case .object(let object) = value else { return nil }
    return object
}

private func garyxBotArrayValue(_ value: GaryxJSONValue?) -> [GaryxJSONValue]? {
    guard case .array(let values) = value else { return nil }
    return values
}

private func garyxBotStringValueIfPresent(_ value: GaryxJSONValue?) -> String? {
    let text = garyxBotStringValue(value).trimmingCharacters(in: .whitespacesAndNewlines)
    return text.isEmpty ? nil : text
}

private func garyxBotStringValue(_ value: GaryxJSONValue?) -> String {
    guard let value else { return "" }
    switch value {
    case .string(let text):
        return text
    case .number(let number):
        if number.rounded() == number {
            return String(Int(number))
        }
        return String(number)
    case .bool(let flag):
        return flag ? "true" : "false"
    case .null:
        return ""
    case .array, .object:
        return ""
    }
}

private func garyxBotBoolValue(_ value: GaryxJSONValue?) -> Bool? {
    guard let value else { return nil }
    switch value {
    case .bool(let flag):
        return flag
    case .string(let text):
        let normalized = text.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if ["true", "yes", "1"].contains(normalized) {
            return true
        }
        if ["false", "no", "0"].contains(normalized) {
            return false
        }
        return nil
    case .number(let number):
        return number != 0
    case .null, .array, .object:
        return nil
    }
}

private extension String {
    var nilIfEmpty: String? {
        isEmpty ? nil : self
    }
}
