import Foundation
import SwiftUI

extension GaryxMobileModel {
    static let workflowRunPollIntervalNanos: UInt64 = 2_000_000_000

    /// Non-idle workflow-panel state — used for cleanup gating
    /// (`showSelectedThread` clears the surface when this is true). NOT the view
    /// branch: a by-id `.resolving` open is not a workflow surface (#TASK-1449
    /// symptom 1 — use `showsWorkflowRunSurface`).
    var isWorkflowRunSurfaceActive: Bool {
        switch workflowRunPanelState.mode {
        case .idle:
            return false
        case .resolving, .run:
            return true
        }
    }

    /// The surface the conversation route should present, as a pure function of
    /// the open thread's objective type (#TASK-1449 symptom 1). A confirmed
    /// workflow-run drilldown (`.run`) presents the workflow surface; an
    /// unclassified by-id open (`.resolving`) is chat-loading, never workflow.
    var conversationSurfaceKind: GaryxConversationSurfaceKind {
        if case .run(let runId) = workflowRunPanelState.mode {
            return .workflowRun(runId: runId)
        }
        let isResolvingById: Bool
        if case .resolving = workflowRunPanelState.mode {
            isResolvingById = true
        } else {
            isResolvingById = false
        }
        return GaryxConversationSurfaceKind.resolve(
            summary: selectedThread ?? selectedWorkflowRunThread,
            isResolvingById: isResolvingById
        )
    }

    /// View branch: present the workflow-run surface only for a confirmed
    /// workflow-run thread.
    var showsWorkflowRunSurface: Bool {
        conversationSurfaceKind.presentsWorkflowRun
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

}
