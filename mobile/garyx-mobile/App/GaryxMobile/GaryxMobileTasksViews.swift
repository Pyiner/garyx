import Foundation
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct GaryxTasksView: View {
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var showsCreateTask = false

    var body: some View {
        GaryxPanelScaffold(
            title: "Tasks",
            subtitle: model.tasksPanelSubtitle,
            onRefresh: { await model.refreshVisibleTasks() },
            contentHorizontalPadding: model.tasksPanelState.isSourceThreadFilterActive ? 0 : 16
        ) {
            VStack(alignment: .leading, spacing: model.tasksPanelState.isSourceThreadFilterActive ? 8 : 14) {
                if model.tasksPanelState.isSourceThreadFilterActive {
                    GaryxTaskSourceThreadFilterRow(
                        title: sourceThreadFilterTitle,
                        threadId: model.tasksPanelState.sourceThreadFilterId ?? "",
                        count: childThreadRows(for: model.visibleTasks).count
                    ) {
                        model.clearTaskSourceThreadFilter()
                    }
                    .padding(.horizontal, 16)
                }
                taskContent
            }
        } actions: {
            GaryxAddToolbarButton(label: "New Task") {
                showsCreateTask = true
            }
        }
        .fullScreenCover(isPresented: $showsCreateTask) {
            GaryxCreateTaskCard()
        }
        .fullScreenCover(item: $model.selectedTaskDetail) { task in
            GaryxFormSheet(title: "Task Details") {
                GaryxTaskDetailCard(task: task)
            }
        }
    }

    @ViewBuilder
    private var taskContent: some View {
        if model.tasksPanelState.isSourceThreadFilterActive {
            switch model.tasksPanelState.sourceThreadFilterLoadPhase {
            case .idle, .loading:
                GaryxLoadingPanelView(title: "Loading source tasks...")
            case .loaded:
                sourceThreadContent(tasks: model.visibleTasks)
            case .failed(let message):
                GaryxEmptyPanelView(
                    icon: "exclamationmark.triangle",
                    title: "Unable to load source tasks.",
                    text: message
                )
            }
        } else if model.tasks.isEmpty, model.isRemoteStatePending {
            GaryxLoadingPanelView(title: "Loading tasks...")
        } else if model.tasks.isEmpty {
            GaryxEmptyPanelView(
                icon: "checklist",
                title: "No tasks yet.",
                text: ""
            )
        } else {
            taskListSection(tasks: model.visibleTasks)
        }
    }

    @ViewBuilder
    private func sourceThreadContent(tasks: [GaryxTaskSummary]) -> some View {
        let rows = childThreadRows(for: tasks)
        if rows.isEmpty {
            GaryxEmptyPanelView(
                icon: "bubble.left.and.text.bubble.right",
                title: "No child threads yet.",
                text: ""
            )
            .padding(.horizontal, 16)
        } else {
            childThreadListSection(rows: rows)
        }
    }

    private func taskListSection(tasks: [GaryxTaskSummary]) -> some View {
        GaryxSectionBlock(title: "Tasks") {
            GaryxCompactListGroup {
                GaryxTaskList(tasks: tasks)
            }
        }
    }

    private func childThreadListSection(rows: [GaryxTaskThreadRowModel]) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            GaryxTaskThreadSectionHeader(title: "Threads")
                .padding(.horizontal, GaryxTaskThreadListMetrics.sectionHorizontalPadding)
                .padding(.bottom, 4)

            ForEach(Array(rows.enumerated()), id: \.element.id) { index, row in
                if index > 0 {
                    GaryxTaskThreadRowDivider()
                }
                GaryxTaskThreadButton(
                    thread: row.thread,
                    isSelected: model.selectedThread?.id == row.thread.id,
                    isPinned: model.isThreadPinned(row.thread.id),
                    trailingTimestamp: row.trailingTimestamp
                ) {
                    Task { await model.openThread(id: row.thread.id) }
                }
            }
        }
    }

    private func childThreadRows(for tasks: [GaryxTaskSummary]) -> [GaryxTaskThreadRowModel] {
        var seenThreadIds = Set<String>()
        return tasks.compactMap { task in
            guard let thread = childThreadSummary(for: task),
                  seenThreadIds.insert(thread.id).inserted else {
                return nil
            }
            return GaryxTaskThreadRowModel(
                thread: thread,
                trailingTimestamp: task.formattedUpdatedAt
            )
        }
    }

    private func childThreadSummary(for task: GaryxTaskSummary) -> GaryxThreadSummary? {
        let threadId = task.threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !threadId.isEmpty else { return nil }
        if let thread = model.sidebarThreadSummary(for: threadId) {
            return thread
        }

        let title = task.title.trimmingCharacters(in: .whitespacesAndNewlines)
        let runtimeAgentId = task.runtimeAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
        return GaryxThreadSummary(
            id: threadId,
            title: title.isEmpty ? "Task thread" : title,
            createdAt: nil,
            updatedAt: task.updatedAt,
            lastMessagePreview: "",
            workspacePath: nil,
            messageCount: nil,
            agentId: runtimeAgentId.isEmpty ? nil : runtimeAgentId,
            teamId: nil,
            teamName: nil,
            providerType: nil,
            recentRunId: nil,
            activeRunId: nil,
            runState: task.status == .inProgress ? "running" : nil,
            worktreePath: nil
        )
    }

    private var sourceThreadFilterTitle: String {
        guard let threadId = model.tasksPanelState.sourceThreadFilterId,
              let thread = model.sidebarThreadSummary(for: threadId) else {
            return "Current thread"
        }
        let title = thread.title.trimmingCharacters(in: .whitespacesAndNewlines)
        return title.isEmpty ? "Current thread" : title
    }
}


private struct GaryxTaskSourceThreadFilterRow: View {
    let title: String
    let threadId: String
    let count: Int
    let onClear: () -> Void

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            Image(systemName: "line.3.horizontal.decrease.circle.fill")
                .font(GaryxFont.system(size: 18, weight: .semibold))
                .foregroundStyle(GaryxTheme.accent)
                .frame(width: 34, height: 34)
                .background(GaryxTheme.accent.opacity(0.10), in: Circle())

            VStack(alignment: .leading, spacing: 2) {
                Text("Child threads from")
                    .font(GaryxFont.caption(weight: .medium))
                    .foregroundStyle(.secondary)
                Text(title)
                    .font(GaryxFont.subheadline(weight: .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                if !threadId.isEmpty {
                    Text(threadId)
                        .font(GaryxFont.system(size: 11, weight: .regular))
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }

            Spacer(minLength: 8)

            Text(countLabel)
                .font(GaryxFont.caption(weight: .semibold))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .fixedSize(horizontal: true, vertical: false)
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(Color.primary.opacity(0.06), in: Capsule())

            Button(action: onClear) {
                GaryxCompactGlassIcon(systemName: "xmark", diameter: 30, iconSize: 12)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Clear source thread filter")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 11)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .fill(GaryxTheme.surface)
        )
        .overlay {
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(GaryxTheme.hairline, lineWidth: 1)
        }
    }

    private var countLabel: String {
        count == 1 ? "1 thread" : "\(count) threads"
    }
}

private enum GaryxTaskThreadListMetrics {
    static let sectionHorizontalPadding: CGFloat = 24
    static let rowOuterPadding: CGFloat = 18
    static let rowInnerHorizontalPadding: CGFloat = 7
}

private struct GaryxTaskThreadRowModel: Identifiable {
    let thread: GaryxThreadSummary
    let trailingTimestamp: String

    var id: String { thread.id }
}

private struct GaryxTaskThreadSectionHeader: View {
    let title: String

    var body: some View {
        Text(title)
            .font(GaryxFont.caption(weight: .medium))
            .foregroundStyle(.secondary)
            .lineLimit(1)
    }
}

private struct GaryxTaskThreadRowDivider: View {
    var body: some View {
        Rectangle()
            .fill(Color.primary.opacity(0.06))
            .frame(height: 1.0 / UIScreen.main.scale)
            .padding(
                .leading,
                (GaryxTaskThreadListMetrics.rowOuterPadding - 4)
                    + GaryxTaskThreadListMetrics.rowInnerHorizontalPadding
                    + 38
                    + 10
            )
            .padding(.trailing, GaryxTaskThreadListMetrics.rowOuterPadding)
            .accessibilityHidden(true)
    }
}

private struct GaryxTaskThreadButton: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let thread: GaryxThreadSummary
    let isSelected: Bool
    let isPinned: Bool
    let trailingTimestamp: String
    let onSelect: () -> Void

    var body: some View {
        GaryxSidebarThreadRowView(
            model: GaryxSidebarThreadRowPresentation(
                thread: thread,
                isSelected: isSelected,
                isPinned: isPinned,
                trailingTimestamp: trailingTimestamp
            ),
            avatar: rowAvatar,
            onSelect: onSelect,
            onUnpin: {
                model.unpinThread(thread.id)
            }
        )
    }

    private var rowAvatar: GaryxSidebarThreadRowAvatar {
        let identity = model.widgetAgentIdentity(for: thread)
        return GaryxSidebarThreadRowAvatar(
            agentId: identity.id ?? "",
            avatarDataUrl: identity.avatarDataUrl ?? "",
            kind: identity.isTeam ? .team : .agent,
            label: identity.name ?? thread.title,
            providerType: identity.providerType ?? "",
            builtIn: identity.builtIn
        )
    }
}

struct GaryxTaskList: View {
    let tasks: [GaryxTaskSummary]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(tasks.enumerated()), id: \.element.id) { index, task in
                GaryxTaskListRow(task: task)
                if index < tasks.count - 1 {
                    GaryxCompactRowDivider()
                }
            }
        }
    }
}

struct GaryxCreateTaskCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    @State private var workspacePath = ""
    @State private var startImmediately = true
    @State private var notificationTargetId = "none"

    var body: some View {
        GaryxFormSheet(
            title: "New Task",
            canSave: canCreate,
            onSave: { Task { await createTask() } }
        ) {
            formContent
        }
        .task {
            await model.refreshAgentTargetsIfNeeded()
        }
        .onAppear {
            workspacePath = model.newThreadWorkspace
        }
    }

    private var formContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            GaryxFormGroupedSection(title: "Task") {
                GaryxFormTextFieldRow(title: "Title", text: $model.draftTaskTitle)
                Divider().padding(.leading, 16)
                GaryxFormTextAreaRow(
                    title: "Details",
                    text: $model.draftTaskBody,
                    minHeight: 128,
                    lineLimits: 3...8
                )
            }

            GaryxFormGroupedSection(title: "Assignee") {
                if model.agentTargets.isEmpty {
                    Text(model.agentTargetsPlaceholderText)
                        .font(GaryxFont.callout())
                        .foregroundStyle(.secondary)
                        .padding(16)
                } else {
                    GaryxFormRow(title: "Agent") {
                        GaryxAgentTargetPickerControl(selectedAgentTargetId: selectedAgentTargetBinding)
                    }
                }
            }

            GaryxFormGroupedSection(title: "Workspace") {
                GaryxWorkspacePathSelectionRow(
                    title: "Workspace",
                    path: $workspacePath,
                    workspacePaths: model.userWorkspacePaths,
                    placeholder: "No workspace",
                    allowsEmpty: true
                )
            }

            GaryxFormGroupedSection(title: "Notification") {
                GaryxFormRow(title: "Target") {
                    Menu {
                        Button {
                            notificationTargetId = "none"
                        } label: {
                            GaryxMenuSelectionLabel(
                                title: "Do not notify",
                                selected: notificationTargetId == "none",
                                fallbackSystemImage: "bell.slash"
                            )
                        }
                        if !model.mobileBotGroups.isEmpty {
                            Divider()
                            ForEach(model.mobileBotGroups) { group in
                                Button {
                                    notificationTargetId = group.id
                                } label: {
                                    GaryxBotGroupMenuSelectionLabel(
                                        group: group,
                                        selected: notificationTargetId == group.id
                                    )
                                }
                            }
                        }
                    } label: {
                        GaryxBotGroupMenuValueLabel(
                            group: selectedNotificationGroup,
                            value: notificationTargetLabel
                        )
                    }
                }
                Divider().padding(.leading, 16)
                GaryxFormRow(title: "Start immediately") {
                    Toggle("Start immediately", isOn: $startImmediately)
                        .labelsHidden()
                }
            }
        }
    }

    private var canCreate: Bool {
        !model.draftTaskTitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !model.draftTaskBody.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var selectedNotificationGroup: GaryxMobileBotGroup? {
        model.mobileBotGroups.first { $0.id == notificationTargetId }
    }

    private var notificationTargetLabel: String {
        selectedNotificationGroup?.title ?? "Do not notify"
    }

    private var notificationTargetRequest: GaryxTaskNotificationTargetRequest {
        guard let group = selectedNotificationGroup else { return .none }
        return .bot(channel: group.channel, accountId: group.accountId)
    }

    private var selectedAgentTargetBinding: Binding<String> {
        Binding {
            model.selectedAgentTargetId
        } set: { value in
            model.setSelectedAgentTarget(value)
        }
    }

    private func createTask() async {
        guard canCreate else { return }
        model.setNewThreadWorkspace(workspacePath)
        await model.createTaskFromDraft(
            start: startImmediately,
            notificationTarget: notificationTargetRequest
        )
        if model.draftTaskTitle.isEmpty, model.draftTaskBody.isEmpty {
            dismiss()
        }
    }
}

struct GaryxTaskListRow: View {
    @EnvironmentObject private var model: GaryxMobileModel
    let task: GaryxTaskSummary
    @State private var showsAssignSheet = false
    @State private var showsDeleteConfirmation = false
    @State private var showsMoreActions = false
    @State private var showsRenamePrompt = false
    @State private var showsStatusActions = false
    @State private var renameDraftTitle = ""

    var body: some View {
        GaryxRowActionMenu(actions: taskSwipeActions) {
            VStack(alignment: .leading, spacing: 8) {
                Button {
                    model.selectedTaskDetail = task
                } label: {
                    (
                        Text(task.displayId)
                            .font(GaryxFont.subheadline(weight: .medium))
                            .foregroundStyle(.secondary)
                        + Text(" ")
                        + Text(task.title)
                            .font(GaryxFont.subheadline(weight: .semibold))
                            .foregroundStyle(.primary)
                    )
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.plain)

                HStack(spacing: 8) {
                    GaryxStatusPill(text: task.status.label, tone: task.status.tone)
                    Text(task.assigneeDisplayLabel)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Spacer(minLength: 8)
                    Text(task.formattedUpdatedAt)
                        .font(GaryxFont.caption())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .contentShape(Rectangle())
        }
        .fullScreenCover(isPresented: $showsAssignSheet) {
            GaryxFormSheet(title: "Assign Task") {
                GaryxTaskAssignCard(task: task)
            }
        }
        .alert("Rename Task", isPresented: $showsRenamePrompt) {
            TextField("Task title", text: $renameDraftTitle)
            Button("Cancel", role: .cancel) {}
            Button("Save") {
                Task { await model.updateTaskTitle(task, title: renameDraftTitle) }
            }
        }
        .confirmationDialog("Task Actions", isPresented: $showsMoreActions, titleVisibility: .visible) {
            Button("Rename") {
                openRenamePrompt()
            }
            Button("Assign") {
                Task { await model.refreshAgentTargetsIfNeeded() }
                showsAssignSheet = true
            }
            Button("Details") {
                model.selectedTaskDetail = task
            }
            if task.assignee != nil || !task.assigneeLabel.isEmpty {
                Button("Unassign") {
                    Task { await model.unassignTask(task) }
                }
            }
            Button("Delete", role: .destructive) {
                showsDeleteConfirmation = true
            }
            Button("Cancel", role: .cancel) {}
        }
        .confirmationDialog("Set Status", isPresented: $showsStatusActions, titleVisibility: .visible) {
            ForEach(task.status.allowedTransitions, id: \.rawValue) { status in
                Button {
                    Task { await model.updateTask(task, to: status) }
                } label: {
                    Label(status.label, systemImage: status.systemImage)
                }
            }
            Button("Cancel", role: .cancel) {}
        }
        .confirmationDialog("Delete task?", isPresented: $showsDeleteConfirmation, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                Task { await model.deleteTask(task) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("This removes the task from the task list.")
        }
    }

    private var taskSwipeActions: [GaryxRowAction] {
        var actions: [GaryxRowAction] = []
        if !task.threadId.isEmpty {
            actions.append(
                GaryxRowAction(title: "Open", systemImage: "message", tone: .accent) {
                    Task { await model.openThread(id: task.threadId) }
                }
            )
        }
        if task.threadId.isEmpty {
            actions.append(
                GaryxRowAction(title: "Details", systemImage: "info.circle", tone: .accent) {
                    model.selectedTaskDetail = task
                }
            )
        }
        if task.status == .inProgress {
            actions.append(
                GaryxRowAction(title: "Stop", systemImage: "stop.fill", tone: .warning) {
                    Task { await model.stopTask(task) }
                }
            )
        }
        actions.append(
            GaryxRowAction(title: "Status", systemImage: "arrow.left.arrow.right.circle") {
                showsStatusActions = true
            }
        )
        actions.append(
            GaryxRowAction(title: "More", systemImage: "ellipsis.circle") {
                showsMoreActions = true
            }
        )
        return actions
    }

    private func openRenamePrompt() {
        renameDraftTitle = task.title
        showsRenamePrompt = true
    }
}

struct GaryxTaskDetailCard: View {
    let task: GaryxTaskSummary

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            GaryxFormGroupedSection(title: "Task") {
                VStack(alignment: .leading, spacing: 8) {
                    HStack(alignment: .firstTextBaseline, spacing: 10) {
                        Text(task.title)
                            .font(GaryxFont.title3(weight: .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(3)
                        Spacer(minLength: 0)
                        GaryxStatusPill(text: task.status.label, tone: task.status.tone)
                    }
                    Text(task.displayId)
                        .font(GaryxFont.caption(weight: .medium))
                        .foregroundStyle(.secondary)
                }
                .padding(16)
            }

            GaryxFormGroupedSection(title: "Details") {
                VStack(spacing: 0) {
                    GaryxTaskMetaLine(label: "Assignee", value: task.assigneeDisplayLabel)
                    Divider().padding(.leading, 16)
                    GaryxTaskMetaLine(label: "Runtime", value: task.runtimeAgentId.isEmpty ? "Not assigned" : task.runtimeAgentId)
                    Divider().padding(.leading, 16)
                    GaryxTaskMetaLine(label: "Thread", value: task.threadId.isEmpty ? "No thread" : task.threadId)
                    Divider().padding(.leading, 16)
                    GaryxTaskMetaLine(label: "Replies", value: "\(task.replyCount)")
                    Divider().padding(.leading, 16)
                    GaryxTaskMetaLine(label: "Updated", value: task.formattedUpdatedAt)
                    if let creator = task.creator {
                        Divider().padding(.leading, 16)
                        GaryxTaskMetaLine(label: "Creator", value: creator.label)
                    }
                    if let updatedBy = task.updatedBy {
                        Divider().padding(.leading, 16)
                        GaryxTaskMetaLine(label: "Updated by", value: updatedBy.label)
                    }
                    if let source = task.source {
                        Divider().padding(.leading, 16)
                        GaryxTaskMetaLine(label: "Source", value: source.detailLabel)
                    }
                }
            }

            if task.threadId.isEmpty {
                GaryxNotice(
                    title: "No chat thread yet",
                    text: "Assign or start this task to create a runnable thread."
                )
            }
        }
    }
}

struct GaryxTaskAssignCard: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: GaryxMobileModel
    let task: GaryxTaskSummary

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            GaryxFormGroupedSection(title: "Assign To") {
                if model.agentTargets.isEmpty, model.shouldShowAgentTargetsEmptyState {
                    GaryxEmptyPanelView(
                        icon: "person.crop.circle.badge.exclamationmark",
                        title: model.agentTargetsEmptyTitle,
                        text: model.agentTargetsEmptyText
                    )
                } else if model.agentTargets.isEmpty {
                    GaryxLoadingPanelView(title: "Loading agents...")
                } else {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(model.agentTargets.enumerated()), id: \.element.id) { index, target in
                            Button {
                                Task {
                                    await model.assignTask(task, agentId: target.id)
                                    dismiss()
                                }
                            } label: {
                                GaryxAgentIdentityRow(
                                    id: target.id,
                                    title: target.title,
                                    subtitle: target.subtitle,
                                    kind: target.kind,
                                    avatarDataUrl: target.avatarDataUrl,
                                    providerType: target.providerType,
                                    builtIn: target.builtIn,
                                    selected: task.assignee?.agentId == target.id
                                        || task.assigneeLabel == target.id
                                        || task.runtimeAgentId == target.id
                                )
                            }
                            .buttonStyle(.plain)
                            if index < model.agentTargets.count - 1 {
                                Divider().padding(.leading, 16)
                            }
                        }
                    }
                }
            }
        }
        .task {
            await model.refreshAgentTargetsIfNeeded()
        }
    }
}

struct GaryxTaskMetaLine: View {
    let label: String
    let value: String

    var body: some View {
        GaryxFormReadOnlyRow(title: label, value: value.isEmpty ? "Unknown" : value)
    }
}


private extension GaryxTaskStatus {
    var label: String {
        switch self {
        case .todo:
            "Todo"
        case .inProgress:
            "In Progress"
        case .inReview:
            "In Review"
        case .done:
            "Done"
        }
    }

    var systemImage: String {
        switch self {
        case .todo:
            "circle"
        case .inProgress:
            "play.circle.fill"
        case .inReview:
            "arrowshape.turn.up.right.circle.fill"
        case .done:
            "checkmark.circle.fill"
        }
    }

    var allowedTransitions: [GaryxTaskStatus] {
        switch self {
        case .todo:
            [.inProgress]
        case .inProgress:
            [.inReview, .todo]
        case .inReview:
            [.done, .inProgress]
        case .done:
            [.todo]
        }
    }

    var tone: GaryxStatusPill.Tone {
        switch self {
        case .todo:
            .muted
        case .inProgress:
            .warning
        case .inReview:
            .danger
        case .done:
            .good
        }
    }
}

private extension GaryxTaskSummary {
    var displayId: String {
        if !id.isEmpty {
            id
        } else if number > 0 {
            "#TASK-\(number)"
        } else {
            "Task"
        }
    }

    var assigneeDisplayLabel: String {
        if let assignee {
            return assignee.garyxDisplayLabel
        }
        if !assigneeLabel.isEmpty {
            return assigneeLabel
        }
        return "Unassigned"
    }

    var formattedUpdatedAt: String {
        garyxFormattedTaskTimestamp(updatedAt)
    }
}

private extension GaryxTaskPrincipal {
    var garyxDisplayLabel: String {
        if kind == "human", let userId, !userId.isEmpty {
            return "@\(userId)"
        }
        if kind == "agent", let agentId, !agentId.isEmpty {
            return agentId
        }
        if let agentId, !agentId.isEmpty {
            return agentId
        }
        if let userId, !userId.isEmpty {
            return "@\(userId)"
        }
        return kind.isEmpty ? "Unknown" : kind
    }

}

private extension GaryxTaskSource {
    var detailLabel: String {
        if let taskId, !taskId.isEmpty {
            return taskId
        }
        if let taskThreadId, !taskThreadId.isEmpty {
            return taskThreadId
        }
        if let threadId, !threadId.isEmpty {
            return threadId
        }
        if let botId, !botId.isEmpty {
            return botId
        }
        let channel = channel ?? ""
        let account = accountId ?? ""
        if !channel.isEmpty, !account.isEmpty {
            return "\(channel) / \(account)"
        }
        if !channel.isEmpty {
            return channel
        }
        return "Unknown"
    }
}
