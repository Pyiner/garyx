import Foundation

/// LRU residency bound for per-thread in-memory projections (TASK-1751 P4).
///
/// The model keeps several unbounded per-thread dictionaries
/// (`messagesByThread`, `messageSignaturesByThread`,
/// `activeAssistantMessageIdsByThread`, `renderSnapshotsByThread`,
/// `cachedTranscriptSnapshots`). Before this bound they only ever released on a
/// gateway switch, so visiting many large threads grew resident memory without
/// limit. This tracker records access order and reports which threads may be
/// evicted so only the current + most-recent threads stay hydrated; evicted
/// state is always re-derivable from the on-disk cache + gateway.
///
/// Pure and side-effect-free: the model owns the dictionaries and performs the
/// actual removals for the ids this returns. `touch` on every per-thread write
/// keeps the access order current.
public struct GaryxThreadResidencyTracker: Equatable, Sendable {
    /// Maximum number of threads kept hydrated in memory (excluding pinned
    /// threads, which are never counted against or evicted by the cap). Six
    /// covers the current thread plus a realistic back-and-forth working set
    /// while bounding worst-case residency.
    public static let defaultMaxResidentThreads = 6

    public let maxResidentThreads: Int
    /// Most-recently-accessed last. Invariant: no duplicates.
    private var accessOrder: [String] = []

    public init(maxResidentThreads: Int = GaryxThreadResidencyTracker.defaultMaxResidentThreads) {
        self.maxResidentThreads = max(1, maxResidentThreads)
    }

    /// Threads currently tracked, most-recently-accessed last (for tests).
    public var residentThreadIds: [String] { accessOrder }

    public var count: Int { accessOrder.count }

    /// Record access to a thread (open, write, stream flush). Moves it to the
    /// most-recent position; a blank id is ignored.
    public mutating func touch(_ threadId: String) {
        let id = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !id.isEmpty else { return }
        if let existing = accessOrder.firstIndex(of: id) {
            accessOrder.remove(at: existing)
        }
        accessOrder.append(id)
    }

    /// Stop tracking a thread (it was deleted, or its projections were cleared
    /// for another reason). Idempotent.
    public mutating func remove(_ threadId: String) {
        let id = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let index = accessOrder.firstIndex(of: id) else { return }
        accessOrder.remove(at: index)
    }

    public mutating func removeAll() {
        accessOrder.removeAll()
    }

    /// The over-cap least-recently-used thread ids that may be evicted, oldest
    /// first, excluding every pinned id. Pinned ids are never evicted and never
    /// counted against the cap, so a working set that is mostly pinned never
    /// forces eviction of a non-pinned thread it still needs.
    ///
    /// Mutating: the returned ids are also dropped from the tracker, so the
    /// caller drops the matching dictionary entries and the tracker stays in
    /// sync with actual residency in one call.
    public mutating func evict(pinned: Set<String>) -> [String] {
        let normalizedPinned = Set(pinned.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) })
        let evictable = accessOrder.filter { !normalizedPinned.contains($0) }
        let overflow = evictable.count - maxResidentThreads
        guard overflow > 0 else { return [] }
        let victims = Array(evictable.prefix(overflow))
        let victimSet = Set(victims)
        accessOrder.removeAll { victimSet.contains($0) }
        return victims
    }
}
