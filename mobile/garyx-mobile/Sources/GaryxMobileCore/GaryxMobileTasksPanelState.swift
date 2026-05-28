import Foundation

public struct GaryxMobileTasksPanelState: Equatable, Sendable {
    public private(set) var sourceThreadFilterId: String?
    public private(set) var sourceThreadFilteredTasks: [GaryxTaskSummary]
    public private(set) var sourceThreadFilterLoadPhase: GaryxMobileLoadPhase

    public init(
        sourceThreadFilterId: String? = nil,
        sourceThreadFilteredTasks: [GaryxTaskSummary] = [],
        sourceThreadFilterLoadPhase: GaryxMobileLoadPhase = .idle
    ) {
        let normalized = Self.normalizedThreadId(sourceThreadFilterId)
        self.sourceThreadFilterId = normalized
        self.sourceThreadFilteredTasks = normalized == nil ? [] : sourceThreadFilteredTasks
        self.sourceThreadFilterLoadPhase = normalized == nil ? .idle : sourceThreadFilterLoadPhase
    }

    public var isSourceThreadFilterActive: Bool {
        sourceThreadFilterId != nil
    }

    public func visibleTasks(from allTasks: [GaryxTaskSummary]) -> [GaryxTaskSummary] {
        isSourceThreadFilterActive ? sourceThreadFilteredTasks : allTasks
    }

    public mutating func setSourceFilter(threadId: String) {
        guard let normalized = Self.normalizedThreadId(threadId) else {
            clearSourceFilter()
            return
        }
        if sourceThreadFilterId != normalized {
            sourceThreadFilteredTasks = []
        }
        sourceThreadFilterId = normalized
        sourceThreadFilterLoadPhase = .loading
    }

    @discardableResult
    public mutating func beginSourceFilterRefresh(threadId: String) -> Bool {
        guard activeFilterMatches(threadId) else { return false }
        sourceThreadFilteredTasks = []
        sourceThreadFilterLoadPhase = .loading
        return true
    }

    @discardableResult
    public mutating func applySourceFilterResult(threadId: String, tasks: [GaryxTaskSummary]) -> Bool {
        guard activeFilterMatches(threadId) else { return false }
        sourceThreadFilteredTasks = tasks
        sourceThreadFilterLoadPhase = .loaded
        return true
    }

    @discardableResult
    public mutating func applySourceFilterFailure(threadId: String, message: String) -> Bool {
        guard activeFilterMatches(threadId) else { return false }
        sourceThreadFilteredTasks = []
        sourceThreadFilterLoadPhase = .failed(message)
        return true
    }

    public mutating func clearSourceFilter() {
        sourceThreadFilterId = nil
        sourceThreadFilteredTasks = []
        sourceThreadFilterLoadPhase = .idle
    }

    public mutating func applyDeletion(taskId: String) {
        let normalized = taskId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return }
        sourceThreadFilteredTasks.removeAll { $0.id == normalized }
    }

    public static func sourceThreadTasks(
        _ tasks: [GaryxTaskSummary],
        sourceThreadId: String?
    ) -> [GaryxTaskSummary] {
        guard let normalized = normalizedThreadId(sourceThreadId) else {
            return tasks
        }
        return tasks.filter { task in
            normalizedThreadId(task.source?.threadId) == normalized
        }
    }

    public static func viewTasksMenuTitle(count: Int) -> String {
        "View Tasks (\(max(0, count)))"
    }

    public static func mergedTasks(
        existing: [GaryxTaskSummary],
        incoming: [GaryxTaskSummary]
    ) -> [GaryxTaskSummary] {
        var incomingById: [String: GaryxTaskSummary] = [:]
        for task in incoming {
            let taskId = task.id.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !taskId.isEmpty else { continue }
            incomingById[taskId] = task
        }

        var consumedIds = Set<String>()
        let replacedExisting = existing.map { task -> GaryxTaskSummary in
            let taskId = task.id.trimmingCharacters(in: .whitespacesAndNewlines)
            guard let replacement = incomingById[taskId] else { return task }
            consumedIds.insert(taskId)
            return replacement
        }
        var appendedIds = Set<String>()
        let newTasks = incoming.filter { task in
            let taskId = task.id.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !taskId.isEmpty else { return true }
            guard !consumedIds.contains(taskId), !appendedIds.contains(taskId) else {
                return false
            }
            appendedIds.insert(taskId)
            return true
        }
        return newTasks + replacedExisting
    }

    private func activeFilterMatches(_ threadId: String) -> Bool {
        guard let active = sourceThreadFilterId,
              let normalized = Self.normalizedThreadId(threadId) else {
            return false
        }
        return active == normalized
    }

    private static func normalizedThreadId(_ value: String?) -> String? {
        let normalized = (value ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        return normalized.isEmpty ? nil : normalized
    }
}
