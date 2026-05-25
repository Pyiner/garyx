import Foundation
import SwiftUI

private func garyxAutomationThreadOptions(
    recentThreads: [GaryxThreadSummary],
    cachedThreads: [GaryxThreadSummary]
) -> [GaryxThreadSummary] {
    var seen = Set<String>()
    return (recentThreads + cachedThreads).filter { seen.insert($0.id).inserted }
}

struct GaryxAutomationsView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateAutomation = false

    var body: some View {
        GaryxAutomationScaffold(title: "Automation") {
            VStack(alignment: .leading, spacing: 16) {
                if model.automations.isEmpty, model.isRemoteStatePending {
                    GaryxLoadingPanelView(title: "Loading automations...")
                } else if model.automations.isEmpty {
                    GaryxEmptyPanelView(
                        icon: "clock.badge",
                        title: "No automations yet. Create your first scheduled prompt.",
                        text: ""
                    )
                } else {
                    VStack(spacing: 14) {
                        ForEach(model.automations) { automation in
                            GaryxAutomationCard(automation: automation)
                        }
                    }
                }
            }
        } trailingAction: {
            Button {
                showsCreateAutomation = true
            } label: {
                GaryxToolbarIcon(systemName: "plus")
            }
            .buttonStyle(.plain)
            .accessibilityLabel("New Automation")
        }
        .fullScreenCover(isPresented: $showsCreateAutomation) {
            GaryxCreateAutomationSheet()
        }
    }
}

struct GaryxAutomationScaffold<Content: View, TrailingAction: View>: View {
    @Environment(\.garyxOpenSidebar) private var openSidebar
    let title: String
    let content: Content
    let trailingAction: TrailingAction

    init(
        title: String,
        @ViewBuilder content: () -> Content,
        @ViewBuilder trailingAction: () -> TrailingAction
    ) {
        self.title = title
        self.content = content()
        self.trailingAction = trailingAction()
    }

    var body: some View {
        ScrollView {
            content
                .padding(.horizontal, 18)
                .padding(.top, 18)
                .padding(.bottom, 32)
                .frame(maxWidth: 560, alignment: .leading)
                .frame(maxWidth: .infinity)
        }
        .background(GaryxAutomationPalette.pageBackground)
        .garyxAdaptiveTopBar {
            ZStack {
                HStack {
                    Button {
                        openSidebar()
                    } label: {
                        GaryxToolbarIcon(systemName: "line.3.horizontal")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Open menu")

                    Spacer(minLength: 0)

                    trailingAction
                }

                Text(title)
                    .font(GaryxFont.title3(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
            }
            .padding(.horizontal, 18)
            .padding(.top, 10)
            .padding(.bottom, 10)
        }
    }
}

struct GaryxAutomationCard: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let automation: GaryxAutomationSummary
    @State private var showsEditForm = false
    @State private var showsDeleteConfirmation = false
    @State private var showsActionPanel = false
    @State private var optimisticEnabled: Bool?

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .top, spacing: 14) {
                Button {
                    openEditForm()
                } label: {
                    VStack(alignment: .leading, spacing: 7) {
                        Text(automation.label)
                            .font(GaryxFont.title3(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                        if !automation.prompt.isEmpty {
                            Text(automation.prompt)
                                .font(GaryxFont.callout())
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.plain)

                Toggle("", isOn: automationEnabledBinding)
                    .labelsHidden()
                    .tint(Color(.systemBlue))
            }

            Divider()
                .overlay(Color.primary.opacity(0.08))

            HStack(alignment: .center, spacing: 10) {
                Text(garyxAutomationScheduleCardSummary(automation.schedule))
                    .font(GaryxFont.callout(weight: .medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Spacer(minLength: 0)
                Button {
                    showsActionPanel.toggle()
                } label: {
                    Image(systemName: "ellipsis")
                        .font(GaryxFont.system(size: 18, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 40, height: 34)
                        .garyxAdaptiveGlass(
                            .regular,
                            isInteractive: true,
                            tint: Color(.systemBackground).opacity(0.66),
                            fallbackMaterial: .ultraThinMaterial,
                            in: Capsule()
                        )
                        .contentShape(Capsule())
                }
                .buttonStyle(GaryxItemActionMenuButtonStyle())
                .accessibilityLabel("Automation actions")
                .popover(isPresented: $showsActionPanel, attachmentAnchor: .rect(.bounds), arrowEdge: .bottom) {
                    GaryxAutomationActionPopover(
                        onRun: {
                            showsActionPanel = false
                            Task {
                                await model.runAutomation(automation)
                            }
                        },
                        onEdit: {
                            showsActionPanel = false
                            openEditForm()
                        },
                        onDelete: {
                            showsActionPanel = false
                            showsDeleteConfirmation = true
                        }
                    )
                    .presentationCompactAdaptation(.popover)
                    .presentationBackground(.ultraThinMaterial)
                    .presentationCornerRadius(22)
                }
            }
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 18)
        .background(GaryxAutomationPalette.cardBackground, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 24, style: .continuous)
                .stroke(Color.primary.opacity(0.05), lineWidth: 1)
        }
        .shadow(color: Color.black.opacity(0.045), radius: 18, x: 0, y: 10)
        .onChange(of: automation.enabled) { _, newValue in
            if optimisticEnabled == newValue {
                optimisticEnabled = nil
            }
        }
        .fullScreenCover(isPresented: $showsEditForm) {
            GaryxEditAutomationSheet(automation: automation)
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

    private var automationEnabledBinding: Binding<Bool> {
        Binding {
            optimisticEnabled ?? automation.enabled
        } set: { nextValue in
            optimisticEnabled = nextValue
            Task {
                if await model.setAutomationEnabled(automation, enabled: nextValue) == false {
                    optimisticEnabled = nil
                }
            }
        }
    }

    private func openEditForm() {
        showsEditForm = true
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
    case .monthly:
        let time = nonEmpty(schedule.time) ?? "09:00"
        let timezone = nonEmpty(schedule.timezone) ?? "UTC"
        return "Monthly on day \(min(max(schedule.day ?? 1, 1), 31)) at \(time) \(timezone)"
    case .once:
        return "Once at \(nonEmpty(schedule.at) ?? "scheduled time")"
    }
}

private func garyxAutomationScheduleCardSummary(_ schedule: GaryxAutomationSchedule) -> String {
    func nonEmpty(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }

    switch schedule.kind {
    case .interval:
        return "Every \(max(1, schedule.hours ?? 24))h"
    case .daily:
        let time = nonEmpty(schedule.time) ?? "09:00"
        let normalizedWeekdays = schedule.weekdays.map { $0.lowercased() }
        if normalizedWeekdays == ["mo", "tu", "we", "th", "fr"] {
            return "Weekdays at \(time)"
        }
        if schedule.weekdays.isEmpty {
            return "Daily at \(time)"
        }
        return "\(schedule.weekdays.map { $0.uppercased() }.joined(separator: ", ")) at \(time)"
    case .monthly:
        let time = nonEmpty(schedule.time) ?? "09:00"
        return "Monthly on \(min(max(schedule.day ?? 1, 1), 31)) at \(time)"
    case .once:
        return "Once at \(nonEmpty(schedule.at) ?? "scheduled time")"
    }
}

private struct GaryxAutomationActionPopover: View {
    let onRun: () -> Void
    let onEdit: () -> Void
    let onDelete: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            actionButton(title: "Run Once", systemName: "clock.arrow.circlepath", action: onRun)
            Divider().padding(.leading, 44)
            actionButton(title: "Edit", systemName: "pencil", action: onEdit)
            Divider().padding(.leading, 44)
            actionButton(title: "Delete", systemName: "trash", isDestructive: true, action: onDelete)
        }
        .frame(width: 226)
        .padding(.vertical, 6)
    }

    private func actionButton(
        title: String,
        systemName: String,
        isDestructive: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(role: isDestructive ? .destructive : nil, action: action) {
            HStack(spacing: 10) {
                Image(systemName: systemName)
                    .font(GaryxFont.system(size: 15, weight: .semibold))
                    .frame(width: 24)
                Text(title)
                    .font(GaryxFont.callout(weight: .medium))
                Spacer(minLength: 0)
            }
            .foregroundStyle(isDestructive ? GaryxTheme.danger : .primary)
            .padding(.horizontal, 12)
            .frame(height: 44)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

struct GaryxCreateAutomationSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var draft = GaryxAutomationDraft()
    @State private var isSaving = false
    @State private var showsThreadPicker = false

    var body: some View {
        GaryxAutomationEditorScaffold(
            title: "New Automation",
            canSave: canCreate && !isSaving,
            onCancel: { dismiss() },
            onSave: save
        ) {
            automationFormFields
        }
        .onAppear(perform: ensureTargetSelection)
        .onChange(of: draft.targetsExistingThread) { _, _ in
            ensureTargetSelection()
        }
        .sheet(isPresented: $showsThreadPicker) {
            GaryxAutomationThreadPickerSheet(
                selectedThreadId: effectiveThreadId,
                onSelect: selectThread
            )
        }
    }

    private var automationFormFields: some View {
        VStack(alignment: .leading, spacing: 22) {
            GaryxAutomationGroupedSection(title: "Title") {
                TextField("Automation name", text: $draft.label)
                    .font(GaryxFont.body())
                    .padding(.horizontal, 16)
                    .frame(minHeight: 52, alignment: .leading)
            }

            GaryxAutomationGroupedSection(title: "Target") {
                Picker("Run In", selection: $draft.targetsExistingThread) {
                    Text("New Thread").tag(false)
                    Text("Existing Thread").tag(true)
                }
                .pickerStyle(.segmented)
                .padding(12)

                Divider().padding(.leading, 16)
                createTargetPicker
            }

            GaryxAutomationGroupedSection(title: "Schedule") {
                GaryxAutomationScheduleEditor(draft: $draft.schedule)
            }

            GaryxAutomationGroupedSection(title: "Prompt") {
                TextField("What should Garyx do?", text: $draft.prompt, axis: .vertical)
                    .font(GaryxFont.body())
                    .lineLimit(5...12)
                    .padding(16)
                    .frame(minHeight: 142, alignment: .topLeading)
            }
        }
    }

    @ViewBuilder
    private var createTargetPicker: some View {
        if draft.targetsExistingThread {
            GaryxAutomationSelectionRow(
                title: "Thread",
                value: selectedThreadTitle,
                placeholder: "Choose recent thread"
            ) {
                showsThreadPicker = true
            }
        } else if workspacePaths.isEmpty {
            GaryxAutomationReadOnlyRow(title: "Workspace", value: "No workspaces available")
        } else {
            GaryxAutomationFormRow(title: "Workspace") {
                Menu {
                    ForEach(workspacePaths, id: \.self) { path in
                        Button {
                            draft.workspacePath = path
                        } label: {
                            Text(path.automationLastPathComponent)
                        }
                    }
                } label: {
                    GaryxAutomationMenuValueLabel(value: effectiveWorkspacePath.automationLastPathComponent)
                }
            }
        }
    }

    private var workspacePaths: [String] {
        model.knownWorkspacePaths
    }

    private var threadOptions: [GaryxThreadSummary] {
        garyxAutomationThreadOptions(recentThreads: model.recentThreads, cachedThreads: model.threads)
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

    private var selectedThreadTitle: String {
        threadOptions
            .first(where: { $0.id == effectiveThreadId })?
            .title ?? effectiveThreadId
    }

    private func ensureTargetSelection() {
        draft.ensureTargetSelection(workspacePaths: workspacePaths, threadOptions: threadOptions)
    }

    private func selectThread(_ thread: GaryxThreadSummary) {
        draft.targetThreadId = thread.id
        if let workspacePath = thread.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines),
           !workspacePath.isEmpty {
            draft.workspacePath = workspacePath
        }
    }

    private func save() {
        guard canCreate, !isSaving else { return }
        isSaving = true
        Task {
            let created = await model.createAutomation(
                label: draft.label,
                prompt: draft.prompt,
                workspacePath: draft.targetsExistingThread ? "" : effectiveWorkspacePath,
                targetThreadId: draft.targetsExistingThread ? effectiveThreadId : "",
                schedule: draft.schedule.schedule
            )
            isSaving = false
            if created {
                dismiss()
            }
        }
    }
}

struct GaryxEditAutomationSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let automation: GaryxAutomationSummary
    @State private var label = ""
    @State private var prompt = ""
    @State private var schedule = GaryxAutomationScheduleDraft()
    @State private var targetsExistingThread = false
    @State private var targetThreadId = ""
    @State private var workspacePath = ""
    @State private var isSaving = false
    @State private var showsThreadPicker = false

    var body: some View {
        GaryxAutomationEditorScaffold(
            title: "Edit Automation",
            canSave: canSave && !isSaving,
            onCancel: { dismiss() },
            onSave: save
        ) {
            VStack(alignment: .leading, spacing: 22) {
                GaryxAutomationGroupedSection(title: "Title") {
                    TextField("Automation name", text: $label)
                        .font(GaryxFont.body())
                        .padding(.horizontal, 16)
                        .frame(minHeight: 52, alignment: .leading)
                }

                GaryxAutomationGroupedSection(title: "Target") {
                    Picker("Run In", selection: $targetsExistingThread) {
                        Text("New Thread").tag(false)
                        Text("Existing Thread").tag(true)
                    }
                    .pickerStyle(.segmented)
                    .padding(12)

                    Divider().padding(.leading, 16)
                    editTargetPicker
                }

                GaryxAutomationGroupedSection(title: "Schedule") {
                    GaryxAutomationScheduleEditor(draft: $schedule)
                }

                GaryxAutomationGroupedSection(title: "Prompt") {
                    TextField("What should Garyx do?", text: $prompt, axis: .vertical)
                        .font(GaryxFont.body())
                        .lineLimit(5...12)
                        .padding(16)
                        .frame(minHeight: 142, alignment: .topLeading)
                }
            }
        }
        .onAppear(perform: fillDraft)
        .onChange(of: targetsExistingThread) { _, _ in
            ensureEditTargetSelection()
        }
        .sheet(isPresented: $showsThreadPicker) {
            GaryxAutomationThreadPickerSheet(
                selectedThreadId: effectiveEditThreadId,
                onSelect: selectThread
            )
        }
    }

    @ViewBuilder
    private var editTargetPicker: some View {
        if targetsExistingThread {
            GaryxAutomationSelectionRow(
                title: "Thread",
                value: selectedThreadTitle,
                placeholder: "Choose recent thread"
            ) {
                showsThreadPicker = true
            }
        } else if editWorkspaceOptions.isEmpty {
            GaryxAutomationReadOnlyRow(title: "Workspace", value: "No workspaces available")
        } else {
            GaryxAutomationFormRow(title: "Workspace") {
                Menu {
                    ForEach(editWorkspaceOptions, id: \.self) { path in
                        Button {
                            workspacePath = path
                        } label: {
                            Text(path.automationLastPathComponent)
                        }
                    }
                } label: {
                    GaryxAutomationMenuValueLabel(value: effectiveEditWorkspacePath.automationLastPathComponent)
                }
            }
        }
    }

    private var editWorkspaceOptions: [String] {
        var seen = Set<String>()
        return ([workspacePath] + model.knownWorkspacePaths)
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .filter { seen.insert($0).inserted }
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
        return editThreadOptions.first?.id ?? ""
    }

    private var canSave: Bool {
        !label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && (targetsExistingThread ? !effectiveEditThreadId.isEmpty : !effectiveEditWorkspacePath.isEmpty)
    }

    private var selectedThreadTitle: String {
        editThreadOptions
            .first(where: { $0.id == effectiveEditThreadId })?
            .title ?? effectiveEditThreadId
    }

    private var editThreadOptions: [GaryxThreadSummary] {
        garyxAutomationThreadOptions(recentThreads: model.recentThreads, cachedThreads: model.threads)
    }

    private func fillDraft() {
        label = automation.label
        prompt = automation.prompt
        schedule = GaryxAutomationScheduleDraft(schedule: automation.schedule)
        let target = automation.targetThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        targetsExistingThread = !target.isEmpty
        targetThreadId = target
        let targetWorkspace = target.isEmpty
            ? ""
            : editThreadOptions.first(where: { $0.id == target })?.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let automationWorkspace = automation.workspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        workspacePath = automationWorkspace.isEmpty ? targetWorkspace : automationWorkspace
        ensureEditTargetSelection()
    }

    private func ensureEditTargetSelection() {
        if targetsExistingThread {
            let nextThreadId = effectiveEditThreadId
            if targetThreadId != nextThreadId {
                targetThreadId = nextThreadId
            }
            if let thread = editThreadOptions.first(where: { $0.id == nextThreadId }),
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

    private func selectThread(_ thread: GaryxThreadSummary) {
        targetThreadId = thread.id
        if let nextWorkspace = thread.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines),
           !nextWorkspace.isEmpty {
            workspacePath = nextWorkspace
        }
    }

    private func save() {
        guard canSave, !isSaving else { return }
        isSaving = true
        Task {
            let updated = await model.updateAutomation(
                automation,
                label: label,
                prompt: prompt,
                schedule: schedule.schedule,
                targetsExistingThread: targetsExistingThread,
                targetThreadId: effectiveEditThreadId,
                workspacePath: effectiveEditWorkspacePath
            )
            isSaving = false
            if updated {
                dismiss()
            }
        }
    }
}

struct GaryxAutomationEditorScaffold<Content: View>: View {
    let title: String
    let canSave: Bool
    let onCancel: () -> Void
    let onSave: () -> Void
    let content: Content

    init(
        title: String,
        canSave: Bool,
        onCancel: @escaping () -> Void,
        onSave: @escaping () -> Void,
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.canSave = canSave
        self.onCancel = onCancel
        self.onSave = onSave
        self.content = content()
    }

    var body: some View {
        GaryxFormSheet(
            title: title,
            canSave: canSave,
            onCancel: onCancel,
            onSave: onSave
        ) {
            content
        }
    }
}

struct GaryxAutomationGroupedSection<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(.secondary)
                .textCase(.uppercase)
                .padding(.horizontal, 14)

            VStack(alignment: .leading, spacing: 0) {
                content
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(GaryxAutomationPalette.cardBackground, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        }
    }
}

struct GaryxAutomationFormRow<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        HStack(spacing: 12) {
            Text(title)
                .font(GaryxFont.body())
                .foregroundStyle(.primary)
            Spacer(minLength: 8)
            content
                .font(GaryxFont.body())
                .foregroundStyle(.primary)
                .multilineTextAlignment(.trailing)
        }
        .padding(.horizontal, 16)
        .frame(minHeight: 52)
    }
}

struct GaryxAutomationReadOnlyRow: View {
    let title: String
    let value: String

    var body: some View {
        GaryxAutomationFormRow(title: title) {
            Text(value)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
        }
    }
}

struct GaryxAutomationMenuValueLabel: View {
    let value: String

    var body: some View {
        HStack(spacing: 6) {
            Text(value)
                .font(GaryxFont.body(weight: .medium))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            Image(systemName: "chevron.up.chevron.down")
                .font(GaryxFont.system(size: 11, weight: .semibold))
                .foregroundStyle(.tertiary)
        }
        .fixedSize(horizontal: false, vertical: true)
    }
}

struct GaryxAutomationScheduleEditor: View {
    @Binding var draft: GaryxAutomationScheduleDraft

    var body: some View {
        VStack(spacing: 0) {
            GaryxAutomationFormRow(title: "Repeat") {
                Menu {
                    ForEach(GaryxAutomationRepeatOption.allCases) { option in
                        Button {
                            draft.repeatOption = option
                        } label: {
                            Text(option.label)
                        }
                    }
                } label: {
                    GaryxAutomationMenuValueLabel(value: draft.repeatOption.label)
                }
            }

            if draft.repeatOption == .interval {
                Divider().padding(.leading, 16)
                GaryxAutomationFormRow(title: "Hours") {
                    Stepper(
                        "\(draft.intervalHours)",
                        value: $draft.intervalHours,
                        in: 1...720
                    )
                    .labelsHidden()
                    Text("\(draft.intervalHours)")
                        .font(GaryxFont.body(weight: .medium))
                        .foregroundStyle(.primary)
                        .frame(minWidth: 32, alignment: .trailing)
                    Text("hours")
                        .font(GaryxFont.body())
                        .foregroundStyle(.secondary)
                }
            }

            if draft.repeatOption == .once {
                Divider().padding(.leading, 16)
                GaryxAutomationFormRow(title: "Date") {
                    DatePicker(
                        "Date",
                        selection: $draft.date,
                        displayedComponents: [.date]
                    )
                    .labelsHidden()
                    .datePickerStyle(.compact)
                    .tint(.secondary)
                }
            }

            if draft.repeatOption == .weekly {
                Divider().padding(.leading, 16)
                GaryxAutomationFormRow(title: "Day") {
                    Menu {
                        ForEach(GaryxAutomationWeekdayOption.allCases) { option in
                            Button {
                                draft.weekday = option.calendarWeekday
                            } label: {
                                Text(option.label)
                            }
                        }
                    } label: {
                        GaryxAutomationMenuValueLabel(value: selectedWeekdayLabel)
                    }
                }
            }

            if draft.repeatOption == .monthly {
                Divider().padding(.leading, 16)
                GaryxAutomationFormRow(title: "Date") {
                    Menu {
                        ForEach(1...31, id: \.self) { day in
                            Button {
                                draft.monthDay = day
                            } label: {
                                Text("\(day)")
                            }
                        }
                    } label: {
                        GaryxAutomationMenuValueLabel(value: "\(draft.monthDay)")
                    }
                }
            }

            if draft.repeatOption != .interval {
                Divider().padding(.leading, 16)
                GaryxAutomationFormRow(title: "Time") {
                    Text(draft.timeString)
                        .font(GaryxFont.body(weight: .medium))
                        .foregroundStyle(.primary)
                        .padding(.horizontal, 12)
                        .frame(height: 34)
                        .background(Color.primary.opacity(0.055), in: Capsule())
                }

                DatePicker(
                    "Time",
                    selection: $draft.time,
                    displayedComponents: [.hourAndMinute]
                )
                .datePickerStyle(.wheel)
                .labelsHidden()
                .frame(maxWidth: .infinity)
                .frame(height: 150)
                .clipped()
                .padding(.horizontal, 22)
                .padding(.bottom, 10)
            }
        }
    }

    private var selectedWeekdayLabel: String {
        GaryxAutomationWeekdayOption.allCases
            .first(where: { $0.calendarWeekday == draft.weekday })?
            .label ?? GaryxAutomationWeekdayOption.monday.label
    }
}

private enum GaryxAutomationWeekdayOption: Int, CaseIterable, Identifiable {
    case sunday = 1
    case monday = 2
    case tuesday = 3
    case wednesday = 4
    case thursday = 5
    case friday = 6
    case saturday = 7

    var id: Int { rawValue }
    var calendarWeekday: Int { rawValue }

    var label: String {
        switch self {
        case .sunday:
            "Sunday"
        case .monday:
            "Monday"
        case .tuesday:
            "Tuesday"
        case .wednesday:
            "Wednesday"
        case .thursday:
            "Thursday"
        case .friday:
            "Friday"
        case .saturday:
            "Saturday"
        }
    }
}

struct GaryxAutomationSelectionRow: View {
    let title: String
    let value: String
    let placeholder: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 12) {
                Text(title)
                    .font(GaryxFont.body())
                    .foregroundStyle(.primary)
                Spacer(minLength: 8)
                Text(displayValue)
                    .font(GaryxFont.body())
                    .foregroundStyle(isPlaceholder ? .secondary : .primary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Image(systemName: "chevron.up.chevron.down")
                    .font(GaryxFont.system(size: 14, weight: .semibold))
                    .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 16)
            .frame(minHeight: 52)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private var displayValue: String {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? placeholder : value
    }

    private var isPlaceholder: Bool {
        value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

struct GaryxAutomationThreadPickerSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let selectedThreadId: String
    let onSelect: (GaryxThreadSummary) -> Void
    @State private var searchText = ""
    @State private var isRefreshing = false

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .center, spacing: 14) {
                Text("Choose thread")
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

            GaryxGlassSearchField("Search threads", text: $searchText)
                .padding(.horizontal, 22)
                .padding(.bottom, 14)

            ScrollView {
                GaryxGlassPanel(cornerRadius: 28, fallbackMaterial: .ultraThinMaterial, shadowOpacity: 0.045) {
                    VStack(spacing: 0) {
                        if indexedFilteredThreads.isEmpty {
                            GaryxAutomationThreadPickerEmptyState(isLoading: isRefreshing)
                        } else {
                            ForEach(indexedFilteredThreads) { item in
                                GaryxAutomationThreadPickerRow(
                                    thread: item.thread,
                                    isSelected: item.thread.id == selectedThreadId,
                                    showsSeparator: item.index < indexedFilteredThreads.count - 1
                                ) {
                                    selectAndClose(item.thread)
                                }
                            }
                        }
                    }
                }
                .padding(.horizontal, 22)
                .padding(.bottom, 28)
            }
            .refreshable {
                await refreshThreadOptions()
            }
            .scrollIndicators(.hidden)
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
        .task {
            await refreshThreadOptions()
        }
    }

    private var indexedFilteredThreads: [GaryxAutomationIndexedThread] {
        Array(filteredThreads.enumerated()).map {
            GaryxAutomationIndexedThread(index: $0.offset, thread: $0.element)
        }
    }

    private var filteredThreads: [GaryxThreadSummary] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let threads = recentThreadOptions
        guard !query.isEmpty else { return threads }
        return threads.filter { thread in
            [
                thread.title,
                thread.workspacePath ?? "",
                thread.agentId ?? "",
                thread.teamName ?? "",
                thread.lastMessagePreview,
            ]
            .contains { $0.lowercased().contains(query) }
        }
    }

    private var recentThreadOptions: [GaryxThreadSummary] {
        var result = garyxAutomationThreadOptions(recentThreads: model.recentThreads, cachedThreads: model.threads)
        let seen = Set(result.map(\.id))
        if !selectedThreadId.isEmpty,
           !seen.contains(selectedThreadId),
           let selected = model.threads.first(where: { $0.id == selectedThreadId }) {
            result.insert(selected, at: 0)
        }
        return result
    }

    private func selectAndClose(_ thread: GaryxThreadSummary) {
        onSelect(thread)
        dismiss()
    }

    private func refreshThreadOptions() async {
        guard !isRefreshing else { return }
        isRefreshing = true
        await model.refreshThreads(silent: true)
        isRefreshing = false
    }
}

private struct GaryxAutomationIndexedThread: Identifiable {
    let index: Int
    let thread: GaryxThreadSummary

    var id: String { thread.id }
}

struct GaryxAutomationThreadPickerRow: View {
    let thread: GaryxThreadSummary
    let isSelected: Bool
    let showsSeparator: Bool
    let onSelect: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            GaryxSidebarThreadRowView(
                model: GaryxSidebarThreadRowPresentation(
                    thread: thread,
                    isSelected: isSelected,
                    isPinned: false,
                    trailingTimestamp: garyxFormattedTaskTimestamp(thread.updatedAt ?? thread.createdAt),
                    showsRunningState: false
                ),
                isFullBleed: true,
                density: .compact,
                selectionDisplay: .checkmark,
                onSelect: onSelect
            )

            if showsSeparator {
                Divider()
                    .padding(.leading, 16)
            }
        }
    }
}

struct GaryxAutomationThreadPickerEmptyState: View {
    let isLoading: Bool

    var body: some View {
        VStack(spacing: 12) {
            if isLoading {
                ProgressView()
                    .controlSize(.regular)
            } else {
                Image(systemName: "bubble.left.and.text.bubble.right")
                    .font(GaryxFont.system(size: 28, weight: .medium))
                    .foregroundStyle(.secondary)
            }
            Text(isLoading ? "Loading recent threads" : "No matching recent threads")
                .font(GaryxFont.callout(weight: .semibold))
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 42)
    }
}

enum GaryxAutomationPalette {
    static let pageBackground = GaryxFormPalette.pageBackground
    static let cardBackground = GaryxFormPalette.cardBackground
}

private extension String {
    var automationLastPathComponent: String {
        (self as NSString).lastPathComponent
    }
}
