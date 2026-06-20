import Foundation

public struct GaryxPendingThreadArchiveState: Equatable, Sendable {
    private var threadIds: Set<String>

    public init(threadIds: Set<String> = []) {
        self.threadIds = Set(threadIds.compactMap(Self.normalizedThreadId))
    }

    public var isEmpty: Bool {
        threadIds.isEmpty
    }

    public mutating func startArchive(threadId: String) {
        guard let normalized = Self.normalizedThreadId(threadId) else { return }
        threadIds.insert(normalized)
    }

    public mutating func resolveArchive(threadId: String) {
        guard let normalized = Self.normalizedThreadId(threadId) else { return }
        threadIds.remove(normalized)
    }

    public func contains(threadId: String) -> Bool {
        guard let normalized = Self.normalizedThreadId(threadId) else { return false }
        return threadIds.contains(normalized)
    }

    public func visibleThreads(_ threads: [GaryxThreadSummary]) -> [GaryxThreadSummary] {
        guard !threadIds.isEmpty else { return threads }
        return threads.filter { !contains(threadId: $0.id) }
    }

    public func visibleThreadIds(_ ids: [String]) -> [String] {
        guard !threadIds.isEmpty else { return ids }
        return ids.filter { !contains(threadId: $0) }
    }

    private static func normalizedThreadId(_ threadId: String) -> String? {
        let normalized = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        return normalized.isEmpty ? nil : normalized
    }
}
