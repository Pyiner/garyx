import Foundation
import SwiftUI

extension GaryxMobileModel {
    static let workflowRunPollIntervalNanos: UInt64 = 2_000_000_000

    var isWorkflowRunSurfaceActive: Bool {
        switch workflowRunPanelState.mode {
        case .idle:
            return false
        case .resolving, .run:
            return true
        }
    }

    func clearWorkflowRunSurface() {
        cancelWorkflowRunPolling()
        workflowRunPanelState.clear()
        selectedWorkflowRunThread = nil
    }

    func showResolvingWorkflowThread(
        threadId: String,
        requestId: UUID,
        source: GaryxMobilePanelOpenSource
    ) {
        guard threadOpenState.markShown(threadId: threadId, requestId: requestId) else { return }
        stopSelectedThreadStream()
        cancelSelectedThreadReconcileLoop()
        selectedThread = nil
        selectedWorkflowRunThread = threads.first(where: { $0.id == threadId })
        workflowRunPanelState.beginResolving(threadId: threadId)
        messages = []
        openConversation(source: source, invalidatesPendingThreadOpen: false)
        lastError = nil
    }

    func openWorkflowRun(
        workflowRunId: String,
        thread: GaryxThreadSummary?,
        invalidatesPendingThreadOpen: Bool,
        source: GaryxMobilePanelOpenSource
    ) async {
        let normalized = workflowRunId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return }
        showWorkflowRun(
            workflowRunId: normalized,
            thread: thread,
            invalidatesPendingThreadOpen: invalidatesPendingThreadOpen,
            source: source
        )
        await refreshWorkflowRun(workflowRunId: normalized)
    }

    func showWorkflowRun(
        workflowRunId: String,
        thread: GaryxThreadSummary?,
        invalidatesPendingThreadOpen: Bool,
        source: GaryxMobilePanelOpenSource
    ) {
        if invalidatesPendingThreadOpen {
            invalidatePendingThreadOpen()
        }
        stopSelectedThreadStream()
        cancelSelectedThreadReconcileLoop()
        if selectedThread != nil {
            selectedThread = nil
        }
        selectedWorkflowRunThread = thread
        workflowRunPanelState.beginRefresh(workflowRunId: workflowRunId)
        messages = []
        draftThreadTitle = thread?.title ?? "Workflow run"
        if let thread {
            persistOpenedThreadDestination(GaryxWorkflowRunDestination.destination(for: thread))
        }
        persistLastSessionRestorable(false)
        openConversation(source: source, invalidatesPendingThreadOpen: false)
        setSidebarVisible(false)
        lastError = nil
    }

    func refreshSelectedWorkflowRun() async {
        guard let workflowRunId = workflowRunPanelState.activeWorkflowRunId else { return }
        await refreshWorkflowRun(workflowRunId: workflowRunId)
    }

    func refreshWorkflowRun(workflowRunId: String) async {
        guard hasGatewaySettings, case .ready = connectionState else { return }
        let normalized = workflowRunId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let drilldown = try await client().getWorkflowRun(workflowRunId: normalized)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            if workflowRunPanelState.applyResult(workflowRunId: normalized, drilldown: drilldown) {
                if let selectedWorkflowRunThread {
                    threads = Self.mergedThreadSummaries(threads + [selectedWorkflowRunThread])
                }
            }
            if drilldown.presentation.terminalComplete {
                cancelWorkflowRunPolling()
            } else {
                startWorkflowRunPollingIfNeeded()
            }
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            let message = displayMessage(for: error)
            _ = workflowRunPanelState.applyFailure(workflowRunId: normalized, message: message)
            lastError = message
        }
    }

    func startWorkflowRunPollingIfNeeded() {
        guard hasGatewaySettings,
              case .ready = connectionState,
              workflowRunPollTask == nil,
              let workflowRunId = workflowRunPanelState.activeWorkflowRunId else {
            return
        }
        let policy = GaryxWorkflowRunPollPolicy.policy(
            presentation: workflowRunPanelState.presentation,
            foregroundVisible: navigationState.presentsContent && navigationState.activePanel == .chat
        )
        guard policy.shouldPoll else { return }
        let generation = UUID()
        workflowRunPollGeneration = generation
        workflowRunPollTask = Task { [weak self] in
            await self?.runWorkflowRunPollLoop(workflowRunId: workflowRunId, generation: generation)
        }
    }

    func cancelWorkflowRunPolling() {
        workflowRunPollTask?.cancel()
        workflowRunPollTask = nil
        workflowRunPollGeneration = nil
    }

    private func runWorkflowRunPollLoop(workflowRunId: String, generation: UUID) async {
        while !Task.isCancelled {
            try? await Task.sleep(nanoseconds: Self.workflowRunPollIntervalNanos)
            if Task.isCancelled { break }
            guard workflowRunPollGeneration == generation,
                  workflowRunPanelState.activeWorkflowRunId == workflowRunId else {
                break
            }
            let policy = GaryxWorkflowRunPollPolicy.policy(
                presentation: workflowRunPanelState.presentation,
                foregroundVisible: navigationState.presentsContent && navigationState.activePanel == .chat
            )
            guard policy.shouldPoll else {
                cancelWorkflowRunPolling()
                break
            }
            await refreshWorkflowRun(workflowRunId: workflowRunId)
        }
    }

    func openTaskThread(_ task: GaryxTaskSummary, source: GaryxMobilePanelOpenSource = .current) async {
        let taskId = task.id.trimmingCharacters(in: .whitespacesAndNewlines)
        let cachedThreadId = task.threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cachedThreadId.isEmpty || !taskId.isEmpty else { return }

        if task.executor?.isWorkflow == true, !taskId.isEmpty {
            let runtimeGeneration = gatewayRuntimeGeneration
            do {
                let refreshed = try await client().getTask(taskId: taskId)
                guard runtimeGeneration == gatewayRuntimeGeneration else { return }
                upsertTask(refreshed)
                let workflowThreadId = refreshed.threadId.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !workflowThreadId.isEmpty else { return }
                await openThread(id: workflowThreadId, source: source)
                return
            } catch {
                guard runtimeGeneration == gatewayRuntimeGeneration else { return }
                lastError = displayMessage(for: error)
            }
        }

        if !cachedThreadId.isEmpty {
            await openThread(id: cachedThreadId, source: source)
        }
    }
}
