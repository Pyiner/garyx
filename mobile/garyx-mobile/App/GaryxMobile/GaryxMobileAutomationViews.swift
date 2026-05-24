import Foundation
import SwiftUI

struct GaryxAutomationsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateAutomation = false
    @State private var activeAutomationActionId: String?

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
                        VStack(spacing: 12) {
                            ForEach(model.automations) { automation in
                                GaryxAutomationCard(
                                    automation: automation,
                                    activeAutomationActionId: $activeAutomationActionId
                                )
                                .zIndex(activeAutomationActionId == automation.id ? 1 : 0)
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
    @Binding var activeAutomationActionId: String?
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false
    @State private var optimisticEnabled: Bool?
    @State private var label = ""
    @State private var prompt = ""
    @State private var intervalHours = ""
    @State private var targetsExistingThread = false
    @State private var targetThreadId = ""
    @State private var workspacePath = ""

    var body: some View {
        ZStack(alignment: .bottomTrailing) {
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .top, spacing: 12) {
                    Button {
                        openEditForm()
                    } label: {
                        VStack(alignment: .leading, spacing: 5) {
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
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .buttonStyle(.plain)

                    Toggle("", isOn: automationEnabledBinding)
                        .labelsHidden()
                        .tint(.blue)
                }

                if !automation.prompt.isEmpty {
                    Button {
                        openEditForm()
                    } label: {
                        Text(automation.prompt)
                            .font(GaryxFont.caption())
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .buttonStyle(.plain)
                }

                Divider()
                    .overlay(GaryxTheme.hairline)

                HStack(alignment: .center, spacing: 10) {
                    Text(garyxAutomationScheduleSummary(automation.schedule))
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    Spacer(minLength: 0)
                    Button {
                        withAnimation(.easeOut(duration: 0.18)) {
                            activeAutomationActionId = showsActionPanel ? nil : automation.id
                        }
                    } label: {
                        Image(systemName: "ellipsis")
                            .font(GaryxFont.system(size: 16, weight: .semibold))
                            .foregroundStyle(.secondary)
                            .frame(width: 34, height: 28)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Automation actions")
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 14)
            .background(GaryxTheme.surface, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 20, style: .continuous)
                    .stroke(GaryxTheme.hairline, lineWidth: 1)
            }

            if showsActionPanel {
                GaryxAutomationActionPanel(
                    canOpenThread: automationOpenThreadId != nil,
                    onRun: {
                        activeAutomationActionId = nil
                        Task {
                            await model.runAutomation(automation)
                        }
                    },
                    onEdit: {
                        activeAutomationActionId = nil
                        openEditForm()
                    },
                    onOpenThread: {
                        guard let threadId = automationOpenThreadId else { return }
                        activeAutomationActionId = nil
                        Task { await model.openThread(id: threadId) }
                    },
                    onDelete: {
                        activeAutomationActionId = nil
                        showsDeleteConfirmation = true
                    }
                )
                .offset(x: -10, y: -38)
                .transition(.scale(scale: 0.94, anchor: .bottomTrailing).combined(with: .opacity))
                .zIndex(2)
            }
        }
        .onAppear(perform: fillDraft)
        .onChange(of: automation.enabled) { _, newValue in
            if optimisticEnabled == newValue {
                optimisticEnabled = nil
            }
        }
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
        VStack(alignment: .leading, spacing: 24) {
            automationEditSummary
            automationEditActions
            automationFields
        }
        .padding(.top, 2)
        .onAppear(perform: fillDraft)
        .onChange(of: targetsExistingThread) { _, _ in
            ensureEditTargetSelection()
        }
    }

    private var automationEditSummary: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .top, spacing: 12) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(automation.label)
                        .font(GaryxFont.title2(weight: .semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)

                    Text(automationTargetLabel)
                        .font(GaryxFont.callout(weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                .frame(maxWidth: .infinity, alignment: .leading)

                GaryxStatusPill(
                    text: automation.enabled ? "Enabled" : "Paused",
                    tone: automation.enabled ? .good : .muted
                )
                .padding(.top, 3)
            }

            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) {
                    automationScheduleChip
                    automationTargetChip
                }
                VStack(alignment: .leading, spacing: 8) {
                    automationScheduleChip
                    automationTargetChip
                }
            }
        }
        .padding(.horizontal, 2)
    }

    private var automationScheduleChip: some View {
        GaryxAutomationInfoChip(systemName: "clock", title: garyxAutomationScheduleSummary(automation.schedule))
    }

    private var automationTargetChip: some View {
        GaryxAutomationInfoChip(
            systemName: targetsExistingThread ? "bubble.left" : "folder",
            title: targetsExistingThread ? "Existing thread" : "New thread"
        )
    }

    private var automationEditActions: some View {
        VStack(alignment: .leading, spacing: 10) {
            if automation.enabled {
                Button {
                    Task {
                        await model.runAutomation(automation)
                        showsEditForm = false
                    }
                } label: {
                    Label("Run Once", systemImage: "play.fill")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(GaryxPrimaryWideButtonStyle())
            }

            HStack(spacing: 10) {
                if let threadId = automationOpenThreadId {
                    GaryxAutomationCommandButton(title: "Thread", systemName: "arrow.up.right") {
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

    private var automationFields: some View {
        VStack(alignment: .leading, spacing: 20) {
            GaryxAutomationFormSection(title: "Details") {
                GaryxAutomationFieldLabel("Name") {
                    TextField("Automation name", text: $label)
                        .garyxInputStyle()
                }
                GaryxAutomationFieldLabel("Prompt") {
                    TextField("What should Garyx run?", text: $prompt, axis: .vertical)
                        .lineLimit(3...8)
                        .garyxInputStyle()
                }
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
                    GaryxAutomationFieldLabel("Repeat every") {
                        HStack(spacing: 10) {
                            TextField("Every", text: $intervalHours)
                                .keyboardType(.numberPad)
                                .garyxInputStyle()
                                .frame(maxWidth: 128)
                            Text("hours")
                                .font(GaryxFont.callout(weight: .medium))
                                .foregroundStyle(.secondary)
                            Spacer(minLength: 0)
                        }
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

    private var showsActionPanel: Bool {
        activeAutomationActionId == automation.id
    }

    private var automationEnabledBinding: Binding<Bool> {
        Binding {
            optimisticEnabled ?? automation.enabled
        } set: { nextValue in
            optimisticEnabled = nextValue
            Task {
                await model.setAutomationEnabled(automation, enabled: nextValue)
            }
        }
    }

    private func openEditForm() {
        activeAutomationActionId = nil
        fillDraft()
        showsEditForm = true
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
        VStack(alignment: .leading, spacing: 12) {
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
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.vertical, 2)
    }
}

struct GaryxAutomationFieldLabel<Content: View>: View {
    let title: String
    let content: Content

    init(_ title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(.tertiary)
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct GaryxAutomationInfoChip: View {
    let systemName: String
    let title: String

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: systemName)
                .font(GaryxFont.system(size: 11, weight: .semibold))
                .foregroundStyle(.secondary)
            Text(title)
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.tail)
        }
        .padding(.horizontal, 9)
        .frame(minHeight: 28)
        .background(Color(.tertiarySystemFill).opacity(0.45), in: Capsule())
    }
}

struct GaryxAutomationActionPanel: View {
    let canOpenThread: Bool
    let onRun: () -> Void
    let onEdit: () -> Void
    let onOpenThread: () -> Void
    let onDelete: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            actionRow(title: "Run Once", systemName: "clock.arrow.circlepath", action: onRun)
            actionRow(title: "Edit Automation", systemName: "pencil", action: onEdit)
            if canOpenThread {
                actionRow(title: "Open Thread", systemName: "arrow.up.right", action: onOpenThread)
            }
            actionRow(title: "Delete Automation", systemName: "trash", isDestructive: true, action: onDelete)
        }
        .padding(.vertical, 8)
        .frame(width: 232)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .stroke(Color.white.opacity(0.42), lineWidth: 1)
        }
        .shadow(color: Color.black.opacity(0.16), radius: 24, x: 0, y: 14)
    }

    private func actionRow(
        title: String,
        systemName: String,
        isDestructive: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Image(systemName: systemName)
                    .font(GaryxFont.system(size: 16, weight: .medium))
                    .frame(width: 22, height: 22)
                Text(title)
                    .font(GaryxFont.callout(weight: .medium))
                Spacer(minLength: 0)
            }
            .foregroundStyle(isDestructive ? GaryxTheme.danger : .primary)
            .padding(.horizontal, 18)
            .frame(height: 48)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
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
            VStack(spacing: 5) {
                Image(systemName: systemName)
                    .font(GaryxFont.system(size: 14, weight: .semibold))
                    .frame(width: 20, height: 18)
                Text(title)
                    .font(GaryxFont.caption(weight: .semibold))
                    .lineLimit(1)
                    .minimumScaleFactor(0.78)
            }
            .foregroundStyle(foreground)
            .frame(maxWidth: .infinity)
            .frame(minHeight: 58)
            .background(background, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 8, style: .continuous)
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
        return Color(.tertiarySystemFill).opacity(0.38)
    }

    private var border: Color {
        isPrimary ? .clear : GaryxTheme.hairline
    }
}

struct GaryxCreateAutomationCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var draft = GaryxAutomationDraft()

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
                Picker("Run In", selection: $draft.targetsExistingThread) {
                    Text("New Thread").tag(false)
                    Text("Existing Thread").tag(true)
                }
                .pickerStyle(.segmented)

                createTargetPicker
            }

            GaryxAutomationFormSection(title: "Prompt") {
                TextField("Name", text: $draft.label)
                    .garyxInputStyle()
                TextField("What should Garyx do?", text: $draft.prompt, axis: .vertical)
                    .lineLimit(4...10)
                    .garyxInputStyle()
            }

            GaryxAutomationFormSection(title: "Schedule") {
                HStack(spacing: 10) {
                    TextField("Every", text: $draft.intervalHours)
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
                    if await model.createAutomation(
                        label: draft.label,
                        prompt: draft.prompt,
                        workspacePath: draft.targetsExistingThread ? "" : effectiveWorkspacePath,
                        targetThreadId: draft.targetsExistingThread ? effectiveThreadId : "",
                        intervalHours: draft.intervalHours
                    ) {
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
        .onChange(of: draft.targetsExistingThread) { _, _ in
            ensureTargetSelection()
        }
    }

    @ViewBuilder
    private var createTargetPicker: some View {
        if draft.targetsExistingThread {
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
            draft.workspacePath = value
        }
    }

    private var threadSelection: Binding<String> {
        Binding {
            effectiveThreadId
        } set: { value in
            draft.targetThreadId = value
            if let thread = model.threads.first(where: { $0.id == value }),
               let workspacePath = thread.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines),
               !workspacePath.isEmpty {
                draft.workspacePath = workspacePath
            }
        }
    }

    private var effectiveWorkspacePath: String {
        draft.effectiveWorkspacePath(workspacePaths: workspacePaths)
    }

    private var effectiveThreadId: String {
        draft.effectiveThreadId(threadOptions: threadOptions)
    }

    private var canCreate: Bool {
        draft.canSubmit(workspacePaths: workspacePaths, threadOptions: threadOptions)
    }

    private func ensureTargetSelection() {
        draft.ensureTargetSelection(workspacePaths: workspacePaths, threadOptions: threadOptions)
    }
}

private extension String {
    var automationLastPathComponent: String {
        (self as NSString).lastPathComponent
    }
}
