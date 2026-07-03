import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    func createTaskFromDraft(
        start: Bool = true,
        notificationTarget: GaryxTaskNotificationTargetRequest = .none
    ) async {
        let title = draftTaskTitle.trimmingCharacters(in: .whitespacesAndNewlines)
        let body = draftTaskBody.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty || !body.isEmpty else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            saveGatewaySettings()
            let target = selectedAgentTargetId.trimmingCharacters(in: .whitespacesAndNewlines)
            let workspace = newThreadWorkspace.trimmingCharacters(in: .whitespacesAndNewlines)
            let task = try await client().createTask(
                GaryxTaskCreateRequest(
                    title: title.isEmpty ? nil : title,
                    body: body.isEmpty ? nil : body,
                    assignee: start && !target.isEmpty ? .agent(target) : nil,
                    start: start,
                    runtime: GaryxTaskRuntimeRequest(
                        agentId: start && !target.isEmpty ? target : nil,
                        workspaceDir: workspace.isEmpty ? nil : workspace
                    ),
                    notificationTarget: notificationTarget
                )
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            draftTaskTitle = ""
            draftTaskBody = ""
            upsertTask(task)
            if !task.threadId.isEmpty {
                await openThread(id: task.threadId)
            }
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func openSelectedThreadTasks() async {
        guard let threadId = selectedThread?.id.trimmingCharacters(in: .whitespacesAndNewlines),
              !threadId.isEmpty else {
            return
        }
        openPanel(.tasks)
        tasksPanelState.setSourceFilter(threadId: threadId)
        await refreshTasksForSourceThread(threadId)
    }

    func clearTaskSourceThreadFilter() {
        tasksPanelState.clearSourceFilter()
    }

    func refreshVisibleTasks() async {
        if let sourceThreadId = tasksPanelState.sourceThreadFilterId {
            await refreshAllTasks()
            await refreshTasksForSourceThread(sourceThreadId)
        } else {
            await refreshAllTasks()
        }
        // Status/title/assign/stop mutations funnel through here; keep the
        // conversation task-tree badge and sidebar snapshot in step.
        noteTaskTreeLocalMutation()
    }

    func refreshAllTasks() async {
        guard hasGatewaySettings else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let page = try await client().listTasks(includeDone: true, limit: 120)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            tasks = page.tasks
            persistCatalogCacheSnapshot()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func refreshTasksForSourceThread(_ sourceThreadId: String) async {
        let normalized = sourceThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return }
        guard tasksPanelState.beginSourceFilterRefresh(threadId: normalized) else { return }
        guard hasGatewaySettings else {
            let message = "Gateway settings are unavailable."
            if tasksPanelState.applySourceFilterFailure(threadId: normalized, message: message) {
                lastError = message
            }
            return
        }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let page = try await client().listTasks(
                filter: GaryxTaskListFilter(
                    sourceThreadId: normalized,
                    includeDone: true,
                    limit: 200
                )
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            if tasksPanelState.applySourceFilterResult(threadId: normalized, tasks: page.tasks) {
                tasks = GaryxMobileTasksPanelState.mergedTasks(existing: tasks, incoming: page.tasks)
                persistCatalogCacheSnapshot()
            }
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            let message = displayMessage(for: error)
            if tasksPanelState.applySourceFilterFailure(threadId: normalized, message: message) {
                lastError = message
            }
        }
    }

    func updateTask(_ task: GaryxTaskSummary, to status: GaryxTaskStatus) async {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            _ = try await client().updateTaskStatus(
                taskId: task.id,
                request: GaryxTaskUpdateStatusRequest(to: status)
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            await refreshVisibleTasks()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func updateTaskTitle(_ task: GaryxTaskSummary, title: String) async {
        let nextTitle = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !nextTitle.isEmpty else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            _ = try await client().updateTaskTitle(taskId: task.id, title: nextTitle)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            await refreshVisibleTasks()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func assignTask(_ task: GaryxTaskSummary, agentId: String) async {
        let target = agentId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !target.isEmpty else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            _ = try await client().assignTask(taskId: task.id, request: GaryxTaskAssignRequest(to: .agent(target)))
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            await refreshVisibleTasks()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func unassignTask(_ task: GaryxTaskSummary) async {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            _ = try await client().unassignTask(taskId: task.id)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            await refreshVisibleTasks()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func stopTask(_ task: GaryxTaskSummary) async {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            _ = try await client().stopTask(taskId: task.id)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            await refreshVisibleTasks()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func deleteTask(_ task: GaryxTaskSummary) async {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            _ = try await client().deleteTask(taskId: task.id)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            tasks.removeAll { $0.id == task.id }
            tasksPanelState.applyDeletion(taskId: task.id)
            persistCatalogCacheSnapshot()
            noteTaskTreeLocalMutation()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func upsertTask(_ task: GaryxTaskSummary) {
        if let index = tasks.firstIndex(where: { $0.id == task.id }) {
            tasks[index] = task
        } else {
            tasks.insert(task, at: 0)
        }
        persistCatalogCacheSnapshot()
        noteTaskTreeLocalMutation()
    }

    func refreshDreams() async {
        guard hasGatewaySettings, dreamsAutoScanEnabled else {
            dreams = []
            latestDreamScan = nil
            return
        }
        do {
            let page = try await client().listDreams(sinceHours: 24, limit: 80)
            dreams = page.dreams
            latestDreamScan = page.scan ?? page.latestScan
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func scanDreams() async {
        guard hasGatewaySettings, dreamsAutoScanEnabled, !isScanningDreams else { return }
        isScanningDreams = true
        defer { isScanningDreams = false }
        do {
            let page = try await client().scanDreams(
                request: GaryxDreamScanRequest(sinceHours: 24, mode: "auto", limit: 600)
            )
            dreams = page.dreams
            latestDreamScan = page.scan ?? page.latestScan
        } catch {
            lastError = displayMessage(for: error)
        }
    }

    func setDreamsAutoScanEnabled(_ enabled: Bool) async {
        guard hasGatewaySettings, dreamsAutoScanEnabled != enabled, !isSavingDreamsSettings else {
            return
        }
        let previous = dreamsAutoScanEnabled
        dreamsAutoScanEnabled = enabled
        isSavingDreamsSettings = true
        defer { isSavingDreamsSettings = false }
        do {
            _ = try await client().saveGatewaySettings([
                "dreams": .object([
                    "enabled": .bool(enabled)
                ])
            ])
            gatewaySettingsStatus = "Saved"
            if !enabled {
                dreams = []
                latestDreamScan = nil
                if activePanel == .dreams {
                    activePanel = .chat
                }
            } else {
                await refreshDreams()
            }
        } catch {
            dreamsAutoScanEnabled = previous
            lastError = displayMessage(for: error)
        }
    }

    func openDreamSpan(_ span: GaryxDreamSpan) async {
        await openThread(id: span.threadId)
    }

    func runAutomation(_ automation: GaryxAutomationSummary) async {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let run = try await client().runAutomationNow(id: automation.id)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastAutomationRun = run
            await refreshRemoteState()
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            if !run.threadId.isEmpty {
                await openThread(id: run.threadId)
            } else if let targetThreadId = automation.targetThreadId, !targetThreadId.isEmpty {
                await openThread(id: targetThreadId)
            }
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func toggleAutomation(_ automation: GaryxAutomationSummary) async {
        await setAutomationEnabled(automation, enabled: !automation.enabled)
    }

    @discardableResult
    func setAutomationEnabled(_ automation: GaryxAutomationSummary, enabled: Bool) async -> Bool {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            _ = try await client().updateAutomationEnabled(
                id: automation.id,
                enabled: enabled
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            await refreshRemoteState()
            return true
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            lastError = displayMessage(for: error)
            return false
        }
    }

    @discardableResult
    func updateAutomation(
        _ automation: GaryxAutomationSummary,
        label: String,
        prompt: String,
        agentId rawAgentId: String,
        schedule: GaryxAutomationSchedule,
        targetsExistingThread: Bool,
        targetThreadId: String,
        workspacePath: String
    ) async -> Bool {
        let nextLabel = label.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextPrompt = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextTargetThreadId = targetsExistingThread
            ? targetThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
            : ""
        let nextAgentId = rawAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
        let nextWorkspacePath = workspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !nextLabel.isEmpty, !nextPrompt.isEmpty else { return false }
        if targetsExistingThread {
            guard !nextTargetThreadId.isEmpty else { return false }
        } else {
            guard !nextAgentId.isEmpty else { return false }
            guard !nextWorkspacePath.isEmpty else { return false }
        }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let updated = try await client().updateAutomation(
                id: automation.id,
                request: GaryxAutomationUpdateRequest(
                    label: nextLabel,
                    prompt: nextPrompt,
                    agentId: nextTargetThreadId.isEmpty ? nextAgentId : nil,
                    workspaceDir: nextTargetThreadId.isEmpty ? nextWorkspacePath : nil,
                    targetThreadId: nextTargetThreadId.isEmpty ? nil : nextTargetThreadId,
                    clearsTargetThreadId: nextTargetThreadId.isEmpty,
                    schedule: schedule
                )
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            replaceAutomation(updated)
            return true
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            lastError = displayMessage(for: error)
            return false
        }
    }


    func createAutomation(
        label rawLabel: String,
        prompt rawPrompt: String,
        agentId rawAgentId: String,
        workspacePath rawWorkspacePath: String,
        targetThreadId rawTargetThreadId: String,
        schedule: GaryxAutomationSchedule
    ) async -> Bool {
        let label = rawLabel.trimmingCharacters(in: .whitespacesAndNewlines)
        let prompt = rawPrompt.trimmingCharacters(in: .whitespacesAndNewlines)
        let agentId = rawAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
        let workspace = rawWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        let targetThreadId = rawTargetThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !label.isEmpty, !prompt.isEmpty else { return false }
        guard !targetThreadId.isEmpty || !workspace.isEmpty else { return false }
        if targetThreadId.isEmpty {
            guard !agentId.isEmpty else { return false }
        }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let automation = try await client().createAutomation(
                GaryxAutomationCreateRequest(
                    label: label,
                    prompt: prompt,
                    agentId: agentId.isEmpty ? nil : agentId,
                    workspaceDir: targetThreadId.isEmpty && !workspace.isEmpty ? workspace : nil,
                    targetThreadId: targetThreadId.isEmpty ? nil : targetThreadId,
                    schedule: schedule,
                    enabled: true
                )
            )
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            automations.insert(automation, at: 0)
            activePanel = .automations
            persistCatalogCacheSnapshot()
            return true
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return false }
            lastError = displayMessage(for: error)
            return false
        }
    }

    func deleteAutomation(_ automation: GaryxAutomationSummary) async {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            _ = try await client().deleteAutomation(id: automation.id)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            automations.removeAll { $0.id == automation.id }
            persistCatalogCacheSnapshot()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func replaceAutomation(_ automation: GaryxAutomationSummary) {
        if let index = automations.firstIndex(where: { $0.id == automation.id }) {
            automations[index] = automation
        } else {
            automations.insert(automation, at: 0)
        }
        persistCatalogCacheSnapshot()
    }
}
