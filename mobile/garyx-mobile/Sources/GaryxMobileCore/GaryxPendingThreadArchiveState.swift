import Foundation

/// Separates remote archive requests from committed runtime tombstones so a
/// failed swipe action never produces a delete-then-reinsert List update.
public struct GaryxPendingThreadArchiveState: Equatable, Sendable {
    private var requestThreadIds: Set<String>
    private var committedThreadIds: Set<String>

    public init() {
        requestThreadIds = []
        committedThreadIds = []
    }

    public var isEmpty: Bool {
        requestThreadIds.isEmpty && committedThreadIds.isEmpty
    }

    /// Starts one remote archive request without changing list visibility.
    /// Returning false coalesces duplicate taps and stale row actions.
    @discardableResult
    public mutating func startArchive(threadId: String) -> Bool {
        guard let normalized = Self.normalizedThreadId(threadId),
              !committedThreadIds.contains(normalized) else {
            return false
        }
        return requestThreadIds.insert(normalized).inserted
    }

    /// Promotes a successful request to a runtime tombstone. Stale async
    /// catalog responses are filtered after the single local list commit,
    /// while an in-flight request alone never removes a visible row.
    public mutating func commitArchive(threadId: String) {
        guard let normalized = Self.normalizedThreadId(threadId) else { return }
        requestThreadIds.remove(normalized)
        committedThreadIds.insert(normalized)
    }

    public mutating func cancelArchive(threadId: String) {
        guard let normalized = Self.normalizedThreadId(threadId) else { return }
        requestThreadIds.remove(normalized)
    }

    public func contains(threadId: String) -> Bool {
        guard let normalized = Self.normalizedThreadId(threadId) else { return false }
        return requestThreadIds.contains(normalized) || committedThreadIds.contains(normalized)
    }

    public func isRequestInFlight(threadId: String) -> Bool {
        guard let normalized = Self.normalizedThreadId(threadId) else { return false }
        return requestThreadIds.contains(normalized)
    }

    public func isCommitted(threadId: String) -> Bool {
        guard let normalized = Self.normalizedThreadId(threadId) else { return false }
        return committedThreadIds.contains(normalized)
    }

    public func visibleThreads(_ threads: [GaryxThreadSummary]) -> [GaryxThreadSummary] {
        guard !committedThreadIds.isEmpty else { return threads }
        return threads.filter { thread in
            guard let normalized = Self.normalizedThreadId(thread.id) else { return true }
            return !committedThreadIds.contains(normalized)
        }
    }

    public func visibleThreadIds(_ ids: [String]) -> [String] {
        guard !committedThreadIds.isEmpty else { return ids }
        return ids.filter { threadId in
            guard let normalized = Self.normalizedThreadId(threadId) else { return true }
            return !committedThreadIds.contains(normalized)
        }
    }

    private static func normalizedThreadId(_ threadId: String) -> String? {
        let normalized = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        return normalized.isEmpty ? nil : normalized
    }
}
