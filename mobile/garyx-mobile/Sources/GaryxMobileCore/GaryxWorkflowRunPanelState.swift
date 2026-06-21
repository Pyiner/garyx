import Foundation

public enum GaryxWorkflowRunDestination: Equatable, Sendable {
    case chat(threadId: String)
    case workflowRun(runId: String)
    case unresolved(threadId: String)

    public static func destination(threadId: String, summary: GaryxThreadSummary?) -> Self {
        let normalizedThreadId = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let summary else {
            return .unresolved(threadId: normalizedThreadId)
        }
        return destination(for: summary, fallbackThreadId: normalizedThreadId)
    }

    public static func destination(for summary: GaryxThreadSummary, fallbackThreadId: String? = nil) -> Self {
        let threadId = summary.id.trimmingCharacters(in: .whitespacesAndNewlines)
        let fallback = fallbackThreadId?.trimmingCharacters(in: .whitespacesAndNewlines)
        let resolvedThreadId = threadId.isEmpty ? fallback ?? "" : threadId
        let threadType = (summary.threadType ?? "").trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        switch threadType {
        case "workflow_run":
            let workflowRunId = (summary.workflowRunId ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            return .workflowRun(runId: workflowRunId.isEmpty ? resolvedThreadId : workflowRunId)
        case "", "unknown":
            return .unresolved(threadId: resolvedThreadId)
        default:
            return .chat(threadId: resolvedThreadId)
        }
    }
}

public enum GaryxWorkflowRunPanelMode: Equatable, Sendable {
    case idle
    case resolving(threadId: String)
    case run(workflowRunId: String)
}

public struct GaryxWorkflowRunPanelState: Equatable, Sendable {
    public private(set) var mode: GaryxWorkflowRunPanelMode
    public private(set) var phase: GaryxMobileLoadPhase
    public private(set) var presentation: GaryxWorkflowPresentation?
    public private(set) var lastAppliedSnapshotVersion: UInt64
    public private(set) var latestEventSeq: UInt64
    public private(set) var lastFailureMessage: String?
    public private(set) var isRefreshing: Bool

    public init(
        mode: GaryxWorkflowRunPanelMode = .idle,
        phase: GaryxMobileLoadPhase = .idle,
        presentation: GaryxWorkflowPresentation? = nil,
        lastAppliedSnapshotVersion: UInt64 = 0,
        latestEventSeq: UInt64 = 0,
        lastFailureMessage: String? = nil,
        isRefreshing: Bool = false
    ) {
        self.mode = mode
        self.phase = phase
        self.presentation = presentation
        self.lastAppliedSnapshotVersion = lastAppliedSnapshotVersion
        self.latestEventSeq = latestEventSeq
        self.lastFailureMessage = lastFailureMessage
        self.isRefreshing = isRefreshing
    }

    public var activeWorkflowRunId: String? {
        if case .run(let workflowRunId) = mode {
            return workflowRunId
        }
        return nil
    }

    public var unresolvedThreadId: String? {
        if case .resolving(let threadId) = mode {
            return threadId
        }
        return nil
    }

    public mutating func beginResolving(threadId: String) {
        let normalized = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return }
        mode = .resolving(threadId: normalized)
        phase = .loading
        presentation = nil
        lastAppliedSnapshotVersion = 0
        latestEventSeq = 0
        lastFailureMessage = nil
        isRefreshing = true
    }

    public mutating func beginRefresh(workflowRunId: String) {
        let normalized = workflowRunId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return }
        if activeWorkflowRunId != normalized {
            presentation = nil
            lastAppliedSnapshotVersion = 0
            latestEventSeq = 0
        }
        mode = .run(workflowRunId: normalized)
        lastFailureMessage = nil
        isRefreshing = true
        switch phase {
        case .idle, .failed:
            phase = .loading
        case .loading, .loaded:
            break
        }
    }

    @discardableResult
    public mutating func applyResult(workflowRunId: String, drilldown: GaryxWorkflowRunDrilldown) -> Bool {
        let normalized = workflowRunId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard activeWorkflowRunId == normalized,
              drilldown.presentation.workflowRunId == normalized else {
            return false
        }
        guard drilldown.presentation.snapshotVersion >= lastAppliedSnapshotVersion else {
            isRefreshing = false
            return false
        }
        presentation = drilldown.presentation
        lastAppliedSnapshotVersion = drilldown.presentation.snapshotVersion
        latestEventSeq = drilldown.presentation.latestEventSeq
        phase = .loaded
        lastFailureMessage = nil
        isRefreshing = false
        return true
    }

    @discardableResult
    public mutating func applyFailure(workflowRunId: String, message: String) -> Bool {
        let normalized = workflowRunId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard activeWorkflowRunId == normalized else { return false }
        lastFailureMessage = message
        isRefreshing = false
        phase = presentation == nil ? .failed(message) : .loaded
        return true
    }

    public mutating func clear() {
        mode = .idle
        phase = .idle
        presentation = nil
        lastAppliedSnapshotVersion = 0
        latestEventSeq = 0
        lastFailureMessage = nil
        isRefreshing = false
    }
}

public struct GaryxWorkflowRunPollPolicy: Equatable, Sendable {
    public var foregroundVisible: Bool
    public var terminalComplete: Bool
    public var latestEventSeq: UInt64

    public init(
        foregroundVisible: Bool,
        terminalComplete: Bool,
        latestEventSeq: UInt64 = 0
    ) {
        self.foregroundVisible = foregroundVisible
        self.terminalComplete = terminalComplete
        self.latestEventSeq = latestEventSeq
    }

    public var shouldPoll: Bool {
        foregroundVisible && !terminalComplete
    }

    public func acceptsEvent(seq: UInt64) -> Bool {
        seq > latestEventSeq
    }

    public static func policy(
        presentation: GaryxWorkflowPresentation?,
        foregroundVisible: Bool
    ) -> Self {
        Self(
            foregroundVisible: foregroundVisible,
            terminalComplete: presentation?.terminalComplete ?? false,
            latestEventSeq: presentation?.latestEventSeq ?? 0
        )
    }
}
