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
        GaryxPanelScaffold(
            title: title,
            subtitle: "",
            background: GaryxFormPalette.pageBackground
        ) {
            content
        } actions: {
            trailingAction
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
        .background(GaryxFormPalette.cardBackground, in: RoundedRectangle(cornerRadius: 24, style: .continuous))
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
        GaryxFormSheet(
            title: "New Automation",
            canSave: canCreate && !isSaving,
            onCancel: { dismiss() },
            onSave: save
        ) {
            GaryxAutomationFormFields(
                draft: $draft,
                workspacePaths: workspacePaths,
                threadOptions: threadOptions,
                showsThreadPicker: $showsThreadPicker
            )
        }
        .onAppear(perform: ensureTargetSelection)
        .task {
            await model.refreshAgentTargetsIfNeeded()
            ensureAgentSelection()
        }
        .onChange(of: draft.targetsExistingThread) { _, _ in
            ensureTargetSelection()
            ensureAgentSelection()
        }
        .onChange(of: model.agentTargets) { _, _ in
            ensureAgentSelection()
        }
        .sheet(isPresented: $showsThreadPicker) {
            GaryxAutomationThreadPickerSheet(
                selectedThreadId: effectiveThreadId,
                onSelect: selectThread
            )
        }
    }

    private var workspacePaths: [String] {
        model.userWorkspacePaths
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

    private var effectiveAgentTargetId: String {
        let selected = draft.trimmedAgentTargetId
        if !selected.isEmpty {
            return selected
        }
        let current = model.selectedAgentTargetId.trimmingCharacters(in: .whitespacesAndNewlines)
        if !current.isEmpty {
            return current
        }
        return model.agentTargets.first?.id ?? ""
    }

    private var canCreate: Bool {
        draft.canSubmit(workspacePaths: workspacePaths, threadOptions: threadOptions)
    }

    private func ensureTargetSelection() {
        draft.ensureTargetSelection(workspacePaths: workspacePaths, threadOptions: threadOptions)
        ensureAgentSelection()
    }

    private func ensureAgentSelection() {
        guard !draft.targetsExistingThread else { return }
        let current = draft.trimmedAgentTargetId
        let validIds = Set(model.agentTargets.map(\.id))
        if !current.isEmpty, validIds.isEmpty || validIds.contains(current) {
            return
        }
        let selected = model.selectedAgentTargetId.trimmingCharacters(in: .whitespacesAndNewlines)
        if !selected.isEmpty, validIds.isEmpty || validIds.contains(selected) {
            draft.agentTargetId = selected
            return
        }
        if let first = model.agentTargets.first {
            draft.agentTargetId = first.id
        }
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
                agentId: draft.targetsExistingThread ? "" : effectiveAgentTargetId,
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
    @State private var draft = GaryxAutomationDraft()
    @State private var isSaving = false
    @State private var showsThreadPicker = false

    var body: some View {
        GaryxFormSheet(
            title: "Edit Automation",
            canSave: canSave && !isSaving,
            onCancel: { dismiss() },
            onSave: save
        ) {
            GaryxAutomationFormFields(
                draft: $draft,
                workspacePaths: editWorkspaceOptions,
                threadOptions: editThreadOptions,
                showsThreadPicker: $showsThreadPicker
            )
        }
        .onAppear(perform: fillDraft)
        .task {
            await model.refreshAgentTargetsIfNeeded()
            ensureEditAgentSelection()
        }
        .onChange(of: draft.targetsExistingThread) { _, _ in
            ensureEditTargetSelection()
            ensureEditAgentSelection()
        }
        .onChange(of: model.agentTargets) { _, _ in
            ensureEditAgentSelection()
        }
        .sheet(isPresented: $showsThreadPicker) {
            GaryxAutomationThreadPickerSheet(
                selectedThreadId: draft.effectiveThreadId(threadOptions: editThreadOptions),
                onSelect: selectThread
            )
        }
    }

    private var editWorkspaceOptions: [String] {
        var seen = Set<String>()
        return ([draft.workspacePath] + model.userWorkspacePaths)
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .filter { seen.insert($0).inserted }
    }

    private var effectiveEditAgentTargetId: String {
        let selected = draft.trimmedAgentTargetId
        if !selected.isEmpty {
            return selected
        }
        let current = model.selectedAgentTargetId.trimmingCharacters(in: .whitespacesAndNewlines)
        if !current.isEmpty {
            return current
        }
        return model.agentTargets.first?.id ?? ""
    }

    private var canSave: Bool {
        draft.canSubmit(workspacePaths: editWorkspaceOptions, threadOptions: editThreadOptions)
    }

    private var editThreadOptions: [GaryxThreadSummary] {
        garyxAutomationThreadOptions(recentThreads: model.recentThreads, cachedThreads: model.threads)
    }

    private func fillDraft() {
        let target = automation.targetThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let targetWorkspace = target.isEmpty
            ? ""
            : editThreadOptions.first(where: { $0.id == target })?.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let automationWorkspace = automation.workspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        draft = GaryxAutomationDraft(
            label: automation.label,
            prompt: automation.prompt,
            agentTargetId: automation.agentId,
            schedule: GaryxAutomationScheduleDraft(schedule: automation.schedule),
            targetsExistingThread: !target.isEmpty,
            targetThreadId: target,
            workspacePath: automationWorkspace.isEmpty ? targetWorkspace : automationWorkspace
        )
        ensureEditTargetSelection()
    }

    private func ensureEditTargetSelection() {
        draft.ensureTargetSelection(workspacePaths: editWorkspaceOptions, threadOptions: editThreadOptions)
        ensureEditAgentSelection()
    }

    private func ensureEditAgentSelection() {
        guard !draft.targetsExistingThread else { return }
        let current = draft.trimmedAgentTargetId
        let validIds = Set(model.agentTargets.map(\.id))
        if !current.isEmpty, validIds.isEmpty || validIds.contains(current) {
            return
        }
        let selected = model.selectedAgentTargetId.trimmingCharacters(in: .whitespacesAndNewlines)
        if !selected.isEmpty, validIds.isEmpty || validIds.contains(selected) {
            draft.agentTargetId = selected
            return
        }
        if let first = model.agentTargets.first {
            draft.agentTargetId = first.id
        }
    }

    private func selectThread(_ thread: GaryxThreadSummary) {
        draft.targetThreadId = thread.id
        if let nextWorkspace = thread.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines),
           !nextWorkspace.isEmpty {
            draft.workspacePath = nextWorkspace
        }
    }

    private func save() {
        guard canSave, !isSaving else { return }
        isSaving = true
        Task {
            let updated = await model.updateAutomation(
                automation,
                label: draft.label,
                prompt: draft.prompt,
                agentId: draft.targetsExistingThread ? "" : effectiveEditAgentTargetId,
                schedule: draft.schedule.schedule,
                targetsExistingThread: draft.targetsExistingThread,
                targetThreadId: draft.effectiveThreadId(threadOptions: editThreadOptions),
                workspacePath: draft.effectiveWorkspacePath(workspacePaths: editWorkspaceOptions)
            )
            isSaving = false
            if updated {
                dismiss()
            }
        }
    }
}

struct GaryxAutomationFormFields: View {
    @Binding var draft: GaryxAutomationDraft
    let workspacePaths: [String]
    let threadOptions: [GaryxThreadSummary]
    @Binding var showsThreadPicker: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            GaryxFormGroupedSection(title: "Title") {
                TextField("Automation name", text: $draft.label)
                    .font(GaryxFont.body())
                    .padding(.horizontal, 16)
                    .frame(minHeight: 52, alignment: .leading)
            }

            GaryxFormGroupedSection(title: "Target") {
                Picker("Run In", selection: $draft.targetsExistingThread) {
                    Text("New Thread").tag(false)
                    Text("Existing Thread").tag(true)
                }
                .pickerStyle(.segmented)
                .padding(12)

                Divider().padding(.leading, 16)
                targetPicker
            }

            GaryxFormGroupedSection(title: "Schedule") {
                GaryxAutomationScheduleEditor(draft: $draft.schedule)
            }

            GaryxFormGroupedSection(title: "Prompt") {
                TextField("What should Garyx do?", text: $draft.prompt, axis: .vertical)
                    .font(GaryxFont.body())
                    .lineLimit(5...12)
                    .padding(16)
                    .frame(minHeight: 142, alignment: .topLeading)
            }
        }
    }

    @ViewBuilder
    private var targetPicker: some View {
        if draft.targetsExistingThread {
            GaryxFormSelectionRow(
                title: "Thread",
                value: selectedThreadTitle,
                placeholder: "Choose recent thread"
            ) {
                showsThreadPicker = true
            }
        } else if workspacePaths.isEmpty {
            agentPicker
            Divider().padding(.leading, 16)
            GaryxFormReadOnlyRow(title: "Workspace", value: "No workspaces available")
        } else {
            agentPicker
            Divider().padding(.leading, 16)
            GaryxFormRow(title: "Workspace") {
                Menu {
                    ForEach(workspacePaths, id: \.self) { path in
                        Button {
                            draft.workspacePath = path
                        } label: {
                            Text(path.automationLastPathComponent)
                        }
                    }
                } label: {
                    GaryxFormMenuValueLabel(
                        value: draft.effectiveWorkspacePath(workspacePaths: workspacePaths).automationLastPathComponent
                    )
                }
            }
        }
    }

    private var agentPicker: some View {
        GaryxAutomationAgentSelectorRow(agentTargetId: $draft.agentTargetId)
    }

    private var selectedThreadTitle: String {
        let threadId = draft.effectiveThreadId(threadOptions: threadOptions)
        return threadOptions
            .first(where: { $0.id == threadId })?
            .title ?? threadId
    }
}

struct GaryxAutomationAgentSelectorRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @Binding var agentTargetId: String

    var body: some View {
        if model.agentTargets.isEmpty {
            GaryxFormReadOnlyRow(title: "Agent", value: model.agentTargetsPlaceholderText)
        } else {
            GaryxFormRow(title: "Agent") {
                GaryxAgentTargetPickerControl(selectedAgentTargetId: $agentTargetId)
            }
        }
    }
}

struct GaryxAutomationScheduleEditor: View {
    @Binding var draft: GaryxAutomationScheduleDraft

    var body: some View {
        VStack(spacing: 0) {
            GaryxFormRow(title: "Repeat") {
                Menu {
                    ForEach(GaryxAutomationRepeatOption.allCases) { option in
                        Button {
                            draft.repeatOption = option
                        } label: {
                            Text(option.label)
                        }
                    }
                } label: {
                    GaryxFormMenuValueLabel(value: draft.repeatOption.label)
                }
            }

            if draft.repeatOption == .interval {
                Divider().padding(.leading, 16)
                GaryxFormRow(title: "Hours") {
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
                GaryxFormRow(title: "Date") {
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
                GaryxFormRow(title: "Day") {
                    Menu {
                        ForEach(GaryxAutomationWeekdayOption.allCases) { option in
                            Button {
                                draft.weekday = option.calendarWeekday
                            } label: {
                                Text(option.label)
                            }
                        }
                    } label: {
                        GaryxFormMenuValueLabel(value: selectedWeekdayLabel)
                    }
                }
            }

            if draft.repeatOption == .monthly {
                Divider().padding(.leading, 16)
                GaryxFormRow(title: "Date") {
                    Menu {
                        ForEach(1...31, id: \.self) { day in
                            Button {
                                draft.monthDay = day
                            } label: {
                                Text("\(day)")
                            }
                        }
                    } label: {
                        GaryxFormMenuValueLabel(value: "\(draft.monthDay)")
                    }
                }
            }

            if draft.repeatOption != .interval {
                Divider().padding(.leading, 16)
                GaryxFormRow(title: "Time") {
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
        GaryxInlineStateView(
            title: isLoading ? "Loading recent threads" : "No matching recent threads",
            icon: "bubble.left.and.text.bubble.right",
            isLoading: isLoading
        )
    }
}

private extension String {
    var automationLastPathComponent: String {
        (self as NSString).lastPathComponent
    }
}
