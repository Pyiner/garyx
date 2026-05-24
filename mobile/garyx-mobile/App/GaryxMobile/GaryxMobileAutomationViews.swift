import Foundation
import SwiftUI

struct GaryxAutomationsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateAutomation = false

    var body: some View {
        GaryxPanelScaffold(
            title: "Automation",
            subtitle: "\(model.enabledAutomationCount) enabled",
            onRefresh: { await model.refreshRemoteState() }
        ) {
            VStack(alignment: .leading, spacing: 18) {
                if let run = model.lastAutomationRun {
                    GaryxNotice(
                        title: "Last run \(run.status)",
                        text: run.excerpt ?? run.threadId
                    )
                }
                if model.automations.isEmpty {
                    GaryxEmptyPanelView(
                        icon: "clock.badge",
                        title: "No automations yet. Create your first scheduled prompt.",
                        text: ""
                    )
                } else {
                    GaryxSectionBlock(title: "Automation") {
                        GaryxCompactListGroup {
                            ForEach(Array(model.automations.enumerated()), id: \.element.id) { index, automation in
                                GaryxAutomationCard(automation: automation)
                                if index < model.automations.count - 1 {
                                    GaryxCompactRowDivider()
                                }
                            }
                        }
                    }
                }
            }
        } actions: {
            GaryxAddToolbarButton(label: "New Automation") {
                showsCreateAutomation = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateAutomation) {
            GaryxFormSheet(title: "New Automation") {
                GaryxCreateAutomationCard()
            }
        }
    }
}

struct GaryxAutomationCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let automation: GaryxAutomationSummary
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false
    @State private var label = ""
    @State private var prompt = ""
    @State private var intervalHours = ""
    @State private var targetsExistingThread = false
    @State private var targetThreadId = ""
    @State private var workspacePath = ""

    var body: some View {
        Button {
            fillDraft()
            showsEditForm = true
        } label: {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .center, spacing: 10) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(automation.label)
                            .font(GaryxFont.body(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        Text(automationTargetLabel)
                            .font(GaryxFont.caption(weight: .medium))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    Spacer()
                    GaryxStatusPill(text: automation.enabled ? "Enabled" : "Paused", tone: automation.enabled ? .good : .muted)
                }
                if !automation.prompt.isEmpty {
                    Text(automation.prompt)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 11)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onAppear(perform: fillDraft)
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxFormSheet(title: "Edit Automation") {
                editAutomationForm
            }
        }
        .confirmationDialog("Delete automation?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task {
                    await model.deleteAutomation(automation)
                    showsEditForm = false
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the scheduled automation and its saved configuration.")
        }
    }

    private var editAutomationForm: some View {
        VStack(alignment: .leading, spacing: 18) {
            GaryxAutomationFormSection(
                title: "Controls",
                subtitle: garyxAutomationScheduleSummary(automation.schedule)
            ) {
                VStack(alignment: .leading, spacing: 12) {
                    HStack(alignment: .firstTextBaseline, spacing: 10) {
                        VStack(alignment: .leading, spacing: 4) {
                            Text(automation.label)
                                .font(GaryxFont.title3(weight: .semibold))
                                .foregroundStyle(.primary)
                                .lineLimit(2)
                            Text(automation.enabled ? "Enabled" : "Paused")
                                .font(GaryxFont.caption(weight: .semibold))
                                .foregroundStyle(.secondary)
                        }
                        Spacer(minLength: 0)
                        GaryxStatusPill(text: automation.enabled ? "Enabled" : "Paused", tone: automation.enabled ? .good : .muted)
                    }

                    HStack(spacing: 10) {
                        if automation.enabled {
                            GaryxAutomationCommandButton(title: "Run Once", systemName: "play.fill", isPrimary: true) {
                                Task {
                                    await model.runAutomation(automation)
                                    showsEditForm = false
                                }
                            }
                        }

                        GaryxAutomationCommandButton(
                            title: automation.enabled ? "Pause" : "Resume",
                            systemName: automation.enabled ? "pause.fill" : "play.fill"
                        ) {
                            Task { await model.toggleAutomation(automation) }
                        }
                    }

                    HStack(spacing: 10) {
                        if let threadId = automationOpenThreadId {
                            GaryxAutomationCommandButton(title: "Open Thread", systemName: "arrow.up.right") {
                                Task {
                                    await model.openThread(id: threadId)
                                    showsEditForm = false
                                }
                            }
                        }

                        GaryxAutomationCommandButton(title: "Delete", systemName: "trash", isDestructive: true) {
                            showsDeleteConfirmation = true
                        }
                    }
                }
            }

            automationFields
        }
        .onAppear(perform: fillDraft)
        .onChange(of: targetsExistingThread) { _, _ in
            ensureEditTargetSelection()
        }
    }

    private var automationFields: some View {
        VStack(alignment: .leading, spacing: 16) {
            GaryxAutomationFormSection(title: "Details") {
                TextField("Name", text: $label)
                    .garyxInputStyle()
                TextField("Prompt", text: $prompt, axis: .vertical)
                    .lineLimit(3...8)
                    .garyxInputStyle()
            }

            GaryxAutomationFormSection(title: "Target") {
                Picker("Run In", selection: $targetsExistingThread) {
                    Text("New Thread").tag(false)
                    Text("Existing Thread").tag(true)
                }
                .pickerStyle(.segmented)

                editTargetPicker
            }

            GaryxAutomationFormSection(title: "Schedule") {
                if automation.schedule.kind == .interval {
                    HStack(spacing: 10) {
                        TextField("Every", text: $intervalHours)
                            .keyboardType(.numberPad)
                            .garyxInputStyle()
                            .frame(maxWidth: 160)
                        Text("hours")
                            .font(GaryxFont.callout(weight: .medium))
                            .foregroundStyle(.secondary)
                        Spacer(minLength: 0)
                    }
                } else {
                    Text(garyxAutomationScheduleSummary(automation.schedule))
                        .font(GaryxFont.callout(weight: .medium))
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 12)
                        .frame(minHeight: 42, alignment: .leading)
                        .background(GaryxTheme.input, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
            }

            Button {
                Task {
                    await model.updateAutomation(
                        automation,
                        label: label,
                        prompt: prompt,
                        intervalHours: intervalHours,
                        targetsExistingThread: targetsExistingThread,
                        targetThreadId: effectiveEditThreadId,
                        workspacePath: effectiveEditWorkspacePath
                    )
                    showsEditForm = false
                }
            } label: {
                Label("Save Changes", systemImage: "checkmark")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(GaryxPrimaryWideButtonStyle())
            .disabled(!canSave)
        }
    }

    @ViewBuilder
    private var editTargetPicker: some View {
        if targetsExistingThread {
            if model.threads.isEmpty && effectiveEditThreadId.isEmpty {
                Text("No existing threads loaded")
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
            } else {
                Picker("Thread", selection: editThreadSelection) {
                    if !targetThreadId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                       !model.threads.contains(where: { $0.id == targetThreadId }) {
                        Text(targetThreadId).tag(targetThreadId)
                    }
                    ForEach(model.threads, id: \.id) { thread in
                        Text(thread.title).tag(thread.id)
                    }
                }
                .pickerStyle(.menu)
                .garyxInputStyle()
            }
            Text("Each run posts the prompt into the selected thread.")
                .font(GaryxFont.caption())
                .foregroundStyle(.secondary)
        } else if editWorkspaceOptions.isEmpty {
            Text("No workspaces available")
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(.secondary)
        } else {
            Picker("Workspace", selection: editWorkspaceSelection) {
                ForEach(editWorkspaceOptions, id: \.self) { path in
                    Text(path.automationLastPathComponent).tag(path)
                }
            }
            .pickerStyle(.menu)
            .garyxInputStyle()
            Text("Each run creates a fresh automation thread in the selected workspace.")
                .font(GaryxFont.caption())
                .foregroundStyle(.secondary)
        }
    }

    private var automationOpenThreadId: String? {
        let target = automation.targetThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !target.isEmpty {
            return target
        }
        let latest = automation.threadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return latest.isEmpty ? nil : latest
    }

    private var automationTargetLabel: String {
        if let targetThreadId = automationOpenThreadId,
           let thread = model.threads.first(where: { $0.id == targetThreadId }) {
            return "Thread · \(thread.title)"
        }
        if let targetThreadId = automationOpenThreadId,
           automation.targetThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) == targetThreadId {
            return "Thread · \(targetThreadId)"
        }
        return automation.workspacePath.isEmpty ? automation.agentId : automation.workspacePath.automationLastPathComponent
    }

    private func fillDraft() {
        label = automation.label
        prompt = automation.prompt
        intervalHours = String(automation.schedule.hours ?? 24)
        let target = automation.targetThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        targetsExistingThread = !target.isEmpty
        targetThreadId = target
        let targetWorkspace = target.isEmpty
            ? ""
            : model.threads.first(where: { $0.id == target })?.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let automationWorkspace = automation.workspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        workspacePath = automationWorkspace.isEmpty ? targetWorkspace : automationWorkspace
        ensureEditTargetSelection()
    }

    private var editWorkspaceOptions: [String] {
        var seen = Set<String>()
        return ([workspacePath] + model.knownWorkspacePaths)
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .filter { seen.insert($0).inserted }
    }

    private var editWorkspaceSelection: Binding<String> {
        Binding {
            effectiveEditWorkspacePath
        } set: { value in
            workspacePath = value
        }
    }

    private var editThreadSelection: Binding<String> {
        Binding {
            effectiveEditThreadId
        } set: { value in
            targetThreadId = value
            if let thread = model.threads.first(where: { $0.id == value }),
               let nextWorkspace = thread.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines),
               !nextWorkspace.isEmpty {
                workspacePath = nextWorkspace
            }
        }
    }

    private var effectiveEditWorkspacePath: String {
        let selected = workspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        if !selected.isEmpty, editWorkspaceOptions.contains(selected) {
            return selected
        }
        return editWorkspaceOptions.first ?? ""
    }

    private var effectiveEditThreadId: String {
        let selected = targetThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
        if !selected.isEmpty {
            return selected
        }
        return model.threads.first?.id ?? ""
    }

    private var canSave: Bool {
        !label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && (targetsExistingThread ? !effectiveEditThreadId.isEmpty : !effectiveEditWorkspacePath.isEmpty)
            && (automation.schedule.kind != .interval || positiveInteger(intervalHours) != nil)
    }

    private func ensureEditTargetSelection() {
        if targetsExistingThread {
            let nextThreadId = effectiveEditThreadId
            if targetThreadId != nextThreadId {
                targetThreadId = nextThreadId
            }
            if let thread = model.threads.first(where: { $0.id == nextThreadId }),
               let nextWorkspace = thread.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines),
               !nextWorkspace.isEmpty {
                workspacePath = nextWorkspace
            }
        } else {
            let nextWorkspace = effectiveEditWorkspacePath
            if workspacePath != nextWorkspace {
                workspacePath = nextWorkspace
            }
        }
    }

    private func positiveInteger(_ value: String) -> Int? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let parsed = Int(trimmed), parsed > 0 else { return nil }
        return parsed
    }
}

private func garyxAutomationScheduleSummary(_ schedule: GaryxAutomationSchedule) -> String {
    func nonEmpty(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }

    switch schedule.kind {
    case .interval:
        return "Every \(max(1, schedule.hours ?? 24)) hours"
    case .daily:
        let time = nonEmpty(schedule.time) ?? "09:00"
        let timezone = nonEmpty(schedule.timezone) ?? "UTC"
        if schedule.weekdays.isEmpty {
            return "Daily at \(time) \(timezone)"
        }
        return "\(schedule.weekdays.map { $0.uppercased() }.joined(separator: ", ")) at \(time) \(timezone)"
    case .once:
        return "Once at \(nonEmpty(schedule.at) ?? "scheduled time")"
    }
}

struct GaryxAutomationFormSection<Content: View>: View {
    let title: String
    var subtitle: String?
    let content: Content

    init(title: String, subtitle: String? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.subtitle = subtitle
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 11) {
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
                    .textCase(.uppercase)
                if let subtitle, !subtitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Text(subtitle)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.tertiary)
                }
            }

            VStack(alignment: .leading, spacing: 10) {
                content
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .stroke(GaryxTheme.hairline, lineWidth: 1)
        }
    }
}

struct GaryxAutomationCommandButton: View {
    let title: String
    let systemName: String
    var isPrimary = false
    var isDestructive = false
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Label(title, systemImage: systemName)
                .font(GaryxFont.footnote(weight: .semibold))
                .lineLimit(1)
                .frame(maxWidth: .infinity)
                .frame(height: 36)
                .foregroundStyle(foreground)
                .background(background, in: Capsule())
                .overlay {
                    Capsule()
                        .stroke(border, lineWidth: 1)
                }
        }
        .buttonStyle(.plain)
    }

    private var foreground: Color {
        if isPrimary {
            return Color(.systemBackground)
        }
        if isDestructive {
            return GaryxTheme.danger
        }
        return .primary
    }

    private var background: Color {
        if isPrimary {
            return Color(.label)
        }
        if isDestructive {
            return GaryxTheme.danger.opacity(0.08)
        }
        return Color(.tertiarySystemFill).opacity(0.5)
    }

    private var border: Color {
        isPrimary ? .clear : GaryxTheme.hairline
    }
}

struct GaryxCreateAutomationCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            VStack(alignment: .leading, spacing: 6) {
                Text("Schedule an agent prompt")
                    .font(GaryxFont.title3(weight: .semibold))
                    .foregroundStyle(.primary)
                Text("Choose where it runs, write the prompt, then set the interval.")
                    .font(GaryxFont.callout())
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal, 2)

            GaryxAutomationFormSection(title: "Target") {
                Picker("Run In", selection: $model.draftAutomationTargetsExistingThread) {
                    Text("New Thread").tag(false)
                    Text("Existing Thread").tag(true)
                }
                .pickerStyle(.segmented)

                createTargetPicker
            }

            GaryxAutomationFormSection(title: "Prompt") {
                TextField("Name", text: $model.draftAutomationLabel)
                    .garyxInputStyle()
                TextField("What should Garyx do?", text: $model.draftAutomationPrompt, axis: .vertical)
                    .lineLimit(4...10)
                    .garyxInputStyle()
            }

            GaryxAutomationFormSection(title: "Schedule") {
                HStack(spacing: 10) {
                    TextField("Every", text: $model.draftAutomationIntervalHours)
                        .keyboardType(.numberPad)
                        .garyxInputStyle()
                        .frame(maxWidth: 150)
                    Text("hours")
                        .font(GaryxFont.callout(weight: .medium))
                        .foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                }
            }

            Button {
                Task {
                    if await model.createAutomationFromDraft() {
                        dismiss()
                    }
                }
            } label: {
                Label("Create Automation", systemImage: "plus")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(GaryxPrimaryWideButtonStyle())
            .disabled(!canCreate)
        }
        .onAppear(perform: ensureTargetSelection)
        .onChange(of: model.draftAutomationTargetsExistingThread) { _, _ in
            ensureTargetSelection()
        }
    }

    @ViewBuilder
    private var createTargetPicker: some View {
        if model.draftAutomationTargetsExistingThread {
            if threadOptions.isEmpty {
                Text("No existing threads loaded")
                    .font(GaryxFont.caption(weight: .semibold))
                    .foregroundStyle(.secondary)
            } else {
                Picker("Thread", selection: threadSelection) {
                    ForEach(threadOptions, id: \.id) { thread in
                        Text(thread.title).tag(thread.id)
                    }
                }
                .pickerStyle(.menu)
                .garyxInputStyle()
            }
        } else if workspacePaths.isEmpty {
            Text("No workspaces available")
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(.secondary)
        } else {
            Picker("Workspace", selection: workspaceSelection) {
                ForEach(workspacePaths, id: \.self) { path in
                    Text(path.automationLastPathComponent).tag(path)
                }
            }
            .pickerStyle(.menu)
            .garyxInputStyle()
        }
    }

    private var workspacePaths: [String] {
        model.knownWorkspacePaths
    }

    private var threadOptions: [GaryxThreadSummary] {
        model.threads
    }

    private var workspaceSelection: Binding<String> {
        Binding {
            effectiveWorkspacePath
        } set: { value in
            model.selectedWorkspacePath = value
        }
    }

    private var threadSelection: Binding<String> {
        Binding {
            effectiveThreadId
        } set: { value in
            model.draftAutomationTargetThreadId = value
            if let thread = model.threads.first(where: { $0.id == value }),
               let workspacePath = thread.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines),
               !workspacePath.isEmpty {
                model.selectedWorkspacePath = workspacePath
            }
        }
    }

    private var effectiveWorkspacePath: String {
        let selected = model.selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        if !selected.isEmpty, workspacePaths.contains(selected) {
            return selected
        }
        return workspacePaths.first ?? ""
    }

    private var effectiveThreadId: String {
        let selected = model.draftAutomationTargetThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
        if !selected.isEmpty, threadOptions.contains(where: { $0.id == selected }) {
            return selected
        }
        return threadOptions.first?.id ?? ""
    }

    private var canCreate: Bool {
        !model.draftAutomationLabel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !model.draftAutomationPrompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && (model.draftAutomationTargetsExistingThread ? !effectiveThreadId.isEmpty : !effectiveWorkspacePath.isEmpty)
            && positiveInteger(model.draftAutomationIntervalHours) != nil
    }

    private func ensureTargetSelection() {
        if model.draftAutomationTargetsExistingThread {
            let nextThreadId = effectiveThreadId
            if model.draftAutomationTargetThreadId != nextThreadId {
                model.draftAutomationTargetThreadId = nextThreadId
            }
            if let thread = model.threads.first(where: { $0.id == nextThreadId }),
               let workspacePath = thread.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines),
               !workspacePath.isEmpty {
                model.selectedWorkspacePath = workspacePath
            }
        } else {
            let nextSelection = effectiveWorkspacePath
            if model.selectedWorkspacePath != nextSelection {
                model.selectedWorkspacePath = nextSelection
            }
        }
    }

    private func positiveInteger(_ value: String) -> Int? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let parsed = Int(trimmed), parsed > 0 else { return nil }
        return parsed
    }
}

private extension String {
    var automationLastPathComponent: String {
        (self as NSString).lastPathComponent
    }
}
