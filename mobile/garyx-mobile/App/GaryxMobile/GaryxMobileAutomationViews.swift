import Foundation
import SwiftUI

private func garyxAutomationThreadOptions(
    recentThreads: [GaryxThreadSummary],
    selectedThread: GaryxThreadSummary? = nil
) -> [GaryxThreadSummary] {
    var seen = Set<String>()
    return ((selectedThread.map { [$0] } ?? []) + recentThreads)
        .filter { seen.insert($0.id).inserted }
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
        .fullScreenCover(item: $model.selectedAutomationEditor) { automation in
            GaryxEditAutomationSheet(automation: automation)
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
                        HStack(spacing: 8) {
                            Text(automation.label)
                                .font(GaryxFont.title3(weight: .semibold))
                                .foregroundStyle(.primary)
                                .lineLimit(1)
                            if automation.validationState == .invalid {
                                GaryxStatusPill(text: "Invalid", tone: .danger)
                            }
                        }
                        if !automation.prompt.isEmpty {
                            Text(automation.prompt)
                                .font(GaryxFont.callout())
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        if automation.validationState == .invalid,
                           let validationError = automation.validationError,
                           !validationError.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            Text(validationError)
                                .font(GaryxFont.caption(weight: .medium))
                                .foregroundStyle(GaryxTheme.danger)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.plain)

                Toggle("", isOn: automationEnabledBinding)
                    .labelsHidden()
                    .tint(GaryxTheme.controlTint)
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
                        onRun: automation.validationState == .valid
                            ? {
                                showsActionPanel = false
                                Task {
                                    await model.runAutomation(automation)
                                }
                            }
                            : nil,
                        onThreads: automation.isGeneratedThreadMode
                            ? {
                                showsActionPanel = false
                                model.openWorkspaceBotsDrilldown(
                                    .automationThreads(automation.id),
                                    source: .current
                                )
                            }
                            : nil,
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
        .confirmationDialog("Delete automation?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task {
                    await model.deleteAutomation(automation)
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
        model.selectedAutomationEditor = automation
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
        // A missing timezone must not be mislabeled as UTC — omit the suffix.
        let timeLabel = nonEmpty(schedule.timezone).map { "\(time) \($0)" } ?? time
        if schedule.weekdays.isEmpty {
            return "Daily at \(timeLabel)"
        }
        return "\(schedule.weekdays.map { $0.uppercased() }.joined(separator: ", ")) at \(timeLabel)"
    case .monthly:
        let time = nonEmpty(schedule.time) ?? "09:00"
        let timeLabel = nonEmpty(schedule.timezone).map { "\(time) \($0)" } ?? time
        return "Monthly on day \(min(max(schedule.day ?? 1, 1), 31)) at \(timeLabel)"
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
    var onRun: (() -> Void)?
    var onThreads: (() -> Void)?
    let onEdit: () -> Void
    let onDelete: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            if let onRun {
                actionButton(title: "Run Once", systemName: "clock.arrow.circlepath", action: onRun)
            }
            if let onThreads {
                if onRun != nil {
                    Divider().padding(.leading, 44)
                }
                actionButton(title: "Threads", systemName: "bubble.left.and.text.bubble.right", action: onThreads)
            }
            if onRun != nil || onThreads != nil {
                Divider().padding(.leading, 44)
            }
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
                targetAgentLabel: targetAgentLabel,
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
                model: model,
                selectedThreadId: effectiveThreadId,
                onSelect: selectThread
            )
        }
    }

    private var workspacePaths: [String] {
        model.userWorkspacePaths
    }

    private var threadOptions: [GaryxThreadSummary] {
        garyxAutomationThreadOptions(
            recentThreads: model.allRecentThreads,
            selectedThread: model.cachedThreadSummary(for: draft.trimmedTargetThreadId)
        )
    }

    private var effectiveWorkspacePath: String {
        draft.effectiveWorkspacePath(workspacePaths: workspacePaths)
    }

    private var effectiveThreadId: String {
        draft.effectiveThreadId(threadOptions: threadOptions)
    }

    private var effectiveAgentTargetId: String {
        draft.trimmedAgentTargetId
    }

    private var canCreate: Bool {
        draft.canSubmit(
            workspacePaths: workspacePaths,
            threadOptions: threadOptions,
            enabledAgentIds: Set(model.agentTargets.map(\.id))
        )
    }

    private var targetAgentLabel: String {
        let threadId = effectiveThreadId
        let thread = threadOptions.first { $0.id == threadId }
        return GaryxAutomationAgentPresentation.followsThreadLabel(
            resolution: thread == nil && !threadId.isEmpty ? .targetMissing : .followThread,
            effectiveAgentId: thread?.agentId,
            agents: model.agents
        )
    }

    private func ensureTargetSelection() {
        draft.ensureTargetSelection(workspacePaths: workspacePaths, threadOptions: threadOptions)
        ensureAgentSelection()
    }

    private func ensureAgentSelection() {
        guard !draft.targetsExistingThread else { return }
        guard draft.trimmedAgentTargetId.isEmpty else { return }
        let validIds = Set(model.agentTargets.map(\.id))
        let effective = model.effectiveDefaultAgentId?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if validIds.contains(effective) {
            draft.agentTargetId = effective
        }
    }

    private func selectThread(_ thread: GaryxThreadSummary) {
        draft.targetThreadId = thread.id
        draft.agentTargetId = thread.agentId?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
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
                targetAgentLabel: targetAgentLabel,
                showsThreadPicker: $showsThreadPicker
            )
        }
        .onAppear(perform: fillDraft)
        .task {
            await model.refreshAgentTargetsIfNeeded()
        }
        .onChange(of: draft.targetsExistingThread) { _, _ in
            ensureEditTargetSelection()
        }
        .sheet(isPresented: $showsThreadPicker) {
            GaryxAutomationThreadPickerSheet(
                model: model,
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

    private var canSave: Bool {
        draft.canSubmit(
            workspacePaths: editWorkspaceOptions,
            threadOptions: editThreadOptions,
            enabledAgentIds: Set(model.agentTargets.map(\.id))
        )
    }

    private var editThreadOptions: [GaryxThreadSummary] {
        garyxAutomationThreadOptions(
            recentThreads: model.allRecentThreads,
            selectedThread: model.cachedThreadSummary(for: draft.trimmedTargetThreadId)
        )
    }

    private var targetAgentLabel: String {
        let threadId = draft.effectiveThreadId(threadOptions: editThreadOptions)
        let originalThreadId = automation.targetThreadId?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if threadId == originalThreadId, !originalThreadId.isEmpty {
            return GaryxAutomationAgentPresentation.followsThreadLabel(
                resolution: automation.agentResolution,
                effectiveAgentId: automation.effectiveAgentId,
                agents: model.agents
            )
        }
        let thread = editThreadOptions.first { $0.id == threadId }
        return GaryxAutomationAgentPresentation.followsThreadLabel(
            resolution: thread == nil && !threadId.isEmpty ? .targetMissing : .followThread,
            effectiveAgentId: thread?.agentId,
            agents: model.agents
        )
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
            agentTargetId: target.isEmpty
                ? automation.agentId ?? ""
                : automation.effectiveAgentId ?? "",
            schedule: GaryxAutomationScheduleDraft(schedule: automation.schedule),
            targetsExistingThread: !target.isEmpty,
            targetThreadId: target,
            workspacePath: automationWorkspace.isEmpty ? targetWorkspace : automationWorkspace,
            originalTargetsExistingThread: !target.isEmpty,
            originalAgentTargetId: target.isEmpty ? automation.agentId ?? "" : automation.effectiveAgentId ?? "",
            agentChanged: false
        )
        ensureEditTargetSelection()
    }

    private func ensureEditTargetSelection() {
        draft.ensureTargetSelection(workspacePaths: editWorkspaceOptions, threadOptions: editThreadOptions)
    }

    private func selectThread(_ thread: GaryxThreadSummary) {
        draft.targetThreadId = thread.id
        if draft.originalTargetsExistingThread != false {
            draft.agentTargetId = thread.agentId?
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        }
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
                agentId: draft.targetsExistingThread ? nil : draft.updateAgentId,
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
    let targetAgentLabel: String
    @Binding var showsThreadPicker: Bool

    var body: some View {
        Group {
            GaryxFormGroupedSection(title: "Automation") {
                GaryxFormTextFieldRow(title: "Name", text: $draft.label)
            }

            GaryxFormGroupedSection(title: "Target") {
                Picker("Run In", selection: $draft.targetsExistingThread) {
                    Text("New Thread").tag(false)
                    Text("Existing Thread").tag(true)
                }
                .pickerStyle(.segmented)
                targetPicker
            }

            GaryxFormGroupedSection(title: "Schedule") {
                GaryxAutomationScheduleEditor(draft: $draft.schedule)
            }

            GaryxFormGroupedSection(title: "Prompt") {
                GaryxFormTextAreaRow(
                    title: "Prompt",
                    text: $draft.prompt,
                    placeholder: "What should Garyx do?",
                    minHeight: 124,
                    lineLimits: 5...12,
                    offersFocusedEditor: true
                )
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
            GaryxFormReadOnlyRow(title: "Agent", value: targetAgentLabel)
        } else {
            agentPicker
            let workspaceBinding = Binding<String>(
                get: { draft.effectiveWorkspacePath(workspacePaths: workspacePaths) },
                set: { draft.workspacePath = $0 }
            )
            GaryxWorkspacePathSelectionRow(
                title: "Workspace",
                path: workspaceBinding,
                workspacePaths: workspacePaths,
                placeholder: "Choose workspace",
                allowsEmpty: false
            )
        }
    }

    private var agentPicker: some View {
        GaryxAutomationAgentSelectorRow(draft: $draft)
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
    @Binding var draft: GaryxAutomationDraft
    @State private var showsAgentPicker = false

    var body: some View {
        if model.agentTargets.isEmpty {
            GaryxFormReadOnlyRow(
                title: "Agent",
                value: draft.trimmedAgentTargetId.isEmpty
                    ? model.agentTargetsPlaceholderText
                    : "\(draft.trimmedAgentTargetId) · unavailable"
            )
            if model.agentTargetsLoadPhase.hasResolved {
                Text("Enable an agent before creating or changing a generated-thread binding.")
                    .font(GaryxFont.caption())
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 4)
            }
        } else {
            GaryxFormRow(title: "Agent", onTap: { showsAgentPicker = true }) {
                GaryxAgentTargetPickerControl(
                    selectedAgentTargetId: agentSelection,
                    isPresented: $showsAgentPicker
                )
            }
        }
    }

    private var agentSelection: Binding<String> {
        Binding {
            draft.agentTargetId
        } set: { id in
            draft.selectAgentTarget(id)
        }
    }
}

struct GaryxAutomationScheduleEditor: View {
    @Binding var draft: GaryxAutomationScheduleDraft
    @State private var showsOnceDatePicker = false

    var body: some View {
        Group {
            GaryxFormMenuRow(title: "Repeat", value: draft.repeatOption.label) {
                ForEach(GaryxAutomationRepeatOption.allCases) { option in
                    Button {
                        draft.repeatOption = option
                    } label: {
                        Text(option.label)
                    }
                }
            }

            if draft.repeatOption == .interval {
                GaryxFormRow(title: "Hours") {
                    GaryxAutomationIntervalStepper(hours: $draft.intervalHours)
                }
            }

            if draft.repeatOption == .once {
                GaryxFormRow(title: "Date", onTap: { showsOnceDatePicker = true }) {
                    GaryxFormMenuValueLabel(value: draft.date.formatted(date: .abbreviated, time: .omitted))
                }
                .popover(isPresented: $showsOnceDatePicker) {
                    DatePicker(
                        "Date",
                        selection: $draft.date,
                        displayedComponents: [.date]
                    )
                    .labelsHidden()
                    .datePickerStyle(.graphical)
                    .tint(GaryxTheme.controlTint)
                    .padding(12)
                    .presentationCompactAdaptation(.popover)
                }
            }

            if draft.repeatOption == .weekly {
                GaryxFormMenuRow(title: "Day", value: selectedWeekdayLabel) {
                    ForEach(GaryxAutomationWeekdayOption.allCases) { option in
                        Button {
                            draft.weekday = option.calendarWeekday
                        } label: {
                            Text(option.label)
                        }
                    }
                }
            }

            if draft.repeatOption == .monthly {
                GaryxFormMenuRow(title: "Date", value: "\(draft.monthDay)") {
                    ForEach(1...31, id: \.self) { day in
                        Button {
                            draft.monthDay = day
                        } label: {
                            Text("\(day)")
                        }
                    }
                }
            }

            if draft.repeatOption != .interval {
                GaryxFormRow(title: "Time") {
                    DatePicker(
                        "Time",
                        selection: $draft.time,
                        displayedComponents: [.hourAndMinute]
                    )
                    .labelsHidden()
                    .datePickerStyle(.compact)
                    .tint(GaryxTheme.controlTint)
                    .fixedSize()
                }
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

private struct GaryxAutomationIntervalStepper: View {
    @Binding var hours: Int
    private let range = 1...720

    var body: some View {
        HStack(spacing: 0) {
            stepButton(systemName: "minus") {
                hours = max(range.lowerBound, hours - 1)
            }
            .disabled(hours <= range.lowerBound)

            Divider()
                .frame(height: 22)

            Text("\(hours)")
                .font(GaryxFont.body(weight: .semibold))
                .foregroundStyle(.primary)
                .monospacedDigit()
                .frame(width: 44, height: 36)

            Divider()
                .frame(height: 22)

            stepButton(systemName: "plus") {
                hours = min(range.upperBound, hours + 1)
            }
            .disabled(hours >= range.upperBound)
        }
        .background(Color(.tertiarySystemFill).opacity(0.72), in: Capsule())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Repeat interval")
        .accessibilityValue("\(hours) hours")
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment:
                hours = min(range.upperBound, hours + 1)
            case .decrement:
                hours = max(range.lowerBound, hours - 1)
            @unknown default:
                break
            }
        }
    }

    private func stepButton(systemName: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: systemName)
                .font(GaryxFont.system(size: 14, weight: .bold))
                .foregroundStyle(.primary)
                .frame(width: 36, height: 36)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

struct GaryxAutomationThreadPickerSheet: View {
    @Environment(\.dismiss) private var dismiss
    let model: GaryxMobileModel
    let selectedThreadId: String
    let onSelect: (GaryxThreadSummary) -> Void
    @StateObject private var owner: GaryxThreadPickerMembershipOwner
    @StateObject private var transportStore: GaryxThreadPickerTransportStore
    @State private var searchText = ""
    @State private var selectedTarget: GaryxThreadSummary?
    @State private var openSwipeActionRowId: String?

    init(
        model: GaryxMobileModel,
        selectedThreadId: String,
        onSelect: @escaping (GaryxThreadSummary) -> Void
    ) {
        self.model = model
        self.selectedThreadId = selectedThreadId
        self.onSelect = onSelect
        _owner = StateObject(
            wrappedValue: GaryxThreadPickerMembershipOwner(
                cache: model.threadSummaryCache,
                leaseOwner: model.threadSummaryLeaseOwner
            )
        )
        _transportStore = StateObject(wrappedValue: GaryxThreadPickerTransportStore())
    }

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

            pickerNotice

            ScrollView {
                GaryxGlassPanel(cornerRadius: 28, fallbackMaterial: .ultraThinMaterial, shadowOpacity: 0.045) {
                    VStack(spacing: 0) {
                        if let selectedTarget {
                            pickerSectionLabel("Selected")
                            GaryxAutomationThreadPickerRow(
                                model: model,
                                favoritesProvider: model.threadFavoritesProvider,
                                thread: selectedTarget,
                                isSelected: true,
                                showsSeparator: !indexedThreads.isEmpty
                            ) {
                                selectAndClose(selectedTarget)
                            }
                        }

                        if indexedThreads.isEmpty {
                            GaryxAutomationThreadPickerEmptyState(
                                isLoading: owner.snapshot.isRefreshing && !owner.snapshot.isPrimed
                            )
                        } else {
                            if selectedTarget != nil {
                                pickerSectionLabel("Results")
                            }
                            ForEach(indexedThreads) { item in
                                GaryxAutomationThreadPickerRow(
                                    model: model,
                                    favoritesProvider: model.threadFavoritesProvider,
                                    thread: item.thread,
                                    isSelected: false,
                                    showsSeparator: item.index < indexedThreads.count - 1
                                ) {
                                    selectAndClose(item.thread)
                                }
                                .onAppear {
                                    if item.thread.id == prefetchThreadId {
                                        transportStore.startLoadMore {
                                            await model.loadMoreThreadPicker(
                                                owner,
                                                trigger: .nearTail
                                            )
                                        }
                                    }
                                }
                            }
                        }

                        pickerFooter
                    }
                }
                .padding(.horizontal, 22)
                .padding(.bottom, 28)
                .garyxVerticalScrollContentWidth()
            }
            .refreshable {
                await model.refreshThreadPicker(owner)
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
        .environment(\.garyxOpenSwipeActionRowId, $openSwipeActionRowId)
        .task(id: searchText) {
            if !searchText.isEmpty {
                try? await Task.sleep(for: .milliseconds(250))
            }
            guard !Task.isCancelled else { return }
            let changed = owner.replaceQuery(searchText)
            guard changed || !owner.snapshot.isPrimed else { return }
            await model.refreshThreadPicker(owner)
        }
        .task(id: selectedThreadId) {
            selectedTarget = await model.hydrateThreadPickerSelectedTarget(
                owner,
                threadId: selectedThreadId
            )
        }
        .onAppear {
            owner.onCancelInstance = { [weak transportStore] _ in
                transportStore?.cancelLoadMore()
            }
        }
        .onDisappear {
            transportStore.cancelLoadMore()
            owner.onCancelInstance = nil
            owner.close()
        }
    }

    private var indexedThreads: [GaryxAutomationIndexedThread] {
        Array(resultThreads.enumerated()).map {
            GaryxAutomationIndexedThread(index: $0.offset, thread: $0.element)
        }
    }

    private var resultThreads: [GaryxThreadSummary] {
        owner.cache.summaries(for: owner.snapshot.orderedThreadIds)
            .filter { $0.id != selectedTarget?.id }
    }

    private var prefetchThreadId: String? {
        GaryxThreadListPageMerge.prefetchTriggerRowId(
            recentIds: resultThreads.map(\.id)
        )
    }

    @ViewBuilder
    private var pickerNotice: some View {
        switch owner.availability {
        case .unsupportedGateway:
            Label("网关版本过旧，请升级", systemImage: "exclamationmark.triangle")
                .font(GaryxFont.caption(weight: .medium))
                .foregroundStyle(.orange)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 24)
                .padding(.bottom, 10)
        case .failed(let message):
            Button {
                Task { await model.refreshThreadPicker(owner) }
            } label: {
                Label(message.isEmpty ? "Could not load threads" : message, systemImage: "arrow.clockwise")
                    .font(GaryxFont.caption(weight: .medium))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 24)
                    .padding(.bottom, 10)
            }
            .buttonStyle(.plain)
        case .ready:
            EmptyView()
        }
    }

    @ViewBuilder
    private var pickerFooter: some View {
        switch owner.snapshot.footerState {
        case .hidden:
            EmptyView()
        case .idle:
            Color.clear
                .frame(height: 44)
                .onAppear {
                    transportStore.startLoadMore {
                        await model.loadMoreThreadPicker(owner, trigger: .footer)
                    }
                }
        case .loading:
            ProgressView()
                .scaleEffect(0.72)
                .frame(maxWidth: .infinity, minHeight: 44)
        case .failed:
            Button("Couldn't load more · Tap to retry") {
                transportStore.startLoadMore {
                    await model.retryLoadMoreThreadPicker(owner)
                }
            }
            .font(GaryxFont.caption(weight: .medium))
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, minHeight: 44)
            .buttonStyle(.plain)
        }
    }

    private func pickerSectionLabel(_ title: String) -> some View {
        Text(title)
            .font(GaryxFont.caption(weight: .semibold))
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.top, 12)
            .padding(.bottom, 6)
    }

    private func selectAndClose(_ thread: GaryxThreadSummary) {
        onSelect(thread)
        dismiss()
    }

}

@MainActor
private final class GaryxThreadPickerTransportStore: ObservableObject {
    private var loadMoreTask: Task<Void, Never>?
    private var loadMoreToken: UUID?

    func startLoadMore(_ operation: @escaping @MainActor () async -> Void) {
        guard loadMoreTask == nil else { return }
        let token = UUID()
        loadMoreToken = token
        loadMoreTask = Task { [weak self] in
            await operation()
            guard let self, loadMoreToken == token else { return }
            loadMoreTask = nil
            loadMoreToken = nil
        }
    }

    func cancelLoadMore() {
        loadMoreTask?.cancel()
        loadMoreTask = nil
        loadMoreToken = nil
    }
}

private struct GaryxAutomationIndexedThread: Identifiable {
    let index: Int
    let thread: GaryxThreadSummary

    var id: String { thread.id }
}

struct GaryxAutomationThreadPickerRow: View {
    @ObservedObject var model: GaryxMobileModel
    @ObservedObject var favoritesProvider: GaryxFavoritesMembershipProvider
    let thread: GaryxThreadSummary
    let isSelected: Bool
    let showsSeparator: Bool
    let onSelect: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            GaryxThreadListRowButton(
                input: GaryxThreadListRowInput(
                    thread: thread,
                    presentation: GaryxSidebarThreadRowPresentation(
                        thread: thread,
                        isSelected: isSelected,
                        isPinned: isPinned,
                        isFavorite: favoritesProvider.state.isPresented(threadId: thread.id),
                        trailingTimestamp: nil,
                        showsRunningState: false
                    ),
                    avatar: rowAvatar,
                    timestampValue: thread.updatedAt ?? thread.createdAt,
                    capabilities: model.liveThreadRowCapabilities(for: thread),
                    isFullBleed: true,
                    density: .compact,
                    selectionDisplay: .checkmark,
                    swipeStyle: .custom,
                    openSource: .current
                ),
                onOpenThread: { _, _ in onSelect() },
                onSetPinned: { threadId, desired in
                    if desired {
                        guard !model.isThreadPinned(threadId) else { return }
                        model.togglePinnedThread(threadId)
                    } else {
                        model.unpinThread(threadId)
                    }
                },
                onSetFavorite: { threadId, desired in
                    model.setThreadFavorite(threadId, desired: desired)
                },
                onArchive: { thread, _ in
                    Task { await model.archiveThread(thread) }
                }
            )
            .equatable()

            if showsSeparator {
                Divider()
                    .padding(.leading, 16)
            }
        }
    }

    private var isPinned: Bool {
        model.isThreadPinned(thread.id)
    }

    private var rowAvatar: GaryxSidebarThreadRowAvatar {
        let identity = model.widgetAgentIdentity(for: thread)
        return GaryxSidebarThreadRowAvatar(
            agentId: identity.id ?? "",
            avatarDataUrl: identity.avatarDataUrl ?? "",
            label: identity.name ?? thread.title,
            providerType: identity.providerType ?? "",
            builtIn: identity.builtIn
        )
    }
}

struct GaryxAutomationThreadPickerEmptyState: View {
    let isLoading: Bool

    var body: some View {
        GaryxInlineStateView(
            title: isLoading ? "Loading threads" : "No matching threads",
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
