import Foundation

/// Gateway-scoped, main-actor summary truth. Membership state machines retain
/// only ids; every row lookup comes through this cache.
@MainActor
public final class GaryxThreadSummaryCache {
    nonisolated public static let defaultUnpinnedCapacity = 500

    private struct Entry {
        var summary: GaryxThreadSummary
        var pinCount: Int
        var lastAccess: UInt64
    }

    /// Reference-typed RAII handle. Construction and invalidation are hidden
    /// in this file so copyable pager/feed values can never acquire a lease.
    @MainActor
    fileprivate final class PinLease {
        private weak var cache: GaryxThreadSummaryCache?
        fileprivate let threadIds: [String]
        private var valid = true

        fileprivate init(cache: GaryxThreadSummaryCache, threadIds: [String]) {
            self.cache = cache
            self.threadIds = threadIds
        }

        fileprivate func invalidate() {
            guard valid else { return }
            valid = false
            cache?.release(threadIds)
            cache = nil
        }

        deinit {
            guard valid else { return }
            let cache = cache
            let threadIds = threadIds
            if Thread.isMainThread {
                MainActor.assumeIsolated {
                    cache?.release(threadIds)
                }
            } else {
                // Global-actor instances may have their last external owner
                // released off-thread. Preserve RAII without asserting a
                // false executor precondition in that teardown edge.
                Task { @MainActor in
                    cache?.release(threadIds)
                }
            }
        }
    }

    public let unpinnedCapacity: Int
    private var entries: [String: Entry] = [:]
    private var clock: UInt64 = 0

    public init(unpinnedCapacity: Int = defaultUnpinnedCapacity) {
        self.unpinnedCapacity = max(0, unpinnedCapacity)
    }

    public var count: Int { entries.count }
    public var pinnedCount: Int { entries.values.filter { $0.pinCount > 0 }.count }
    public var unpinnedCount: Int { entries.values.filter { $0.pinCount == 0 }.count }

    public func summary(for rawThreadId: String) -> GaryxThreadSummary? {
        guard let threadId = Self.normalizedId(rawThreadId), var entry = entries[threadId] else {
            return nil
        }
        entry.lastAccess = tick()
        entries[threadId] = entry
        return entry.summary
    }

    public func summaries(for rawThreadIds: [String]) -> [GaryxThreadSummary] {
        Self.uniqueIds(rawThreadIds).compactMap(summary(for:))
    }

    public func writeThrough(_ summaries: [GaryxThreadSummary]) {
        writeWithoutPruning(summaries)
        pruneUnpinnedPool()
    }

    public func remove(_ rawThreadId: String) {
        guard let threadId = Self.normalizedId(rawThreadId),
              entries[threadId]?.pinCount == 0 else { return }
        entries[threadId] = nil
    }

    fileprivate func makeLease(
        threadIds rawThreadIds: [String],
        writing summaries: [GaryxThreadSummary]
    ) -> PinLease? {
        // Writes and ref increments form one synchronous transaction. This is
        // what lets a 501+ page become resident without the first rows being
        // evicted between write-through and pin acquisition.
        writeWithoutPruning(summaries)
        let threadIds = Self.uniqueIds(rawThreadIds).filter { entries[$0] != nil }
        guard !threadIds.isEmpty else {
            pruneUnpinnedPool()
            return nil
        }
        for threadId in threadIds {
            guard var entry = entries[threadId] else { continue }
            entry.pinCount += 1
            entry.lastAccess = tick()
            entries[threadId] = entry
        }
        pruneUnpinnedPool()
        return PinLease(cache: self, threadIds: threadIds)
    }

    fileprivate func removeAll() {
        entries.removeAll(keepingCapacity: false)
        clock &+= 1
    }

    func pinCount(for rawThreadId: String) -> Int {
        guard let threadId = Self.normalizedId(rawThreadId) else { return 0 }
        return entries[threadId]?.pinCount ?? 0
    }

    private func writeWithoutPruning(_ summaries: [GaryxThreadSummary]) {
        for summary in summaries {
            guard let threadId = Self.normalizedId(summary.id) else { continue }
            let pinCount = entries[threadId]?.pinCount ?? 0
            entries[threadId] = Entry(
                summary: summary,
                pinCount: pinCount,
                lastAccess: tick()
            )
        }
    }

    private func release(_ threadIds: [String]) {
        for threadId in threadIds {
            guard var entry = entries[threadId], entry.pinCount > 0 else { continue }
            entry.pinCount -= 1
            entry.lastAccess = tick()
            entries[threadId] = entry
        }
        pruneUnpinnedPool()
    }

    private func pruneUnpinnedPool() {
        let candidates = entries
            .filter { $0.value.pinCount == 0 }
            .sorted {
                if $0.value.lastAccess != $1.value.lastAccess {
                    return $0.value.lastAccess < $1.value.lastAccess
                }
                return $0.key < $1.key
            }
        let excess = candidates.count - unpinnedCapacity
        guard excess > 0 else { return }
        for (threadId, _) in candidates.prefix(excess) {
            entries[threadId] = nil
        }
    }

    private func tick() -> UInt64 {
        clock &+= 1
        return clock
    }

    private static func normalizedId(_ rawId: String) -> String? {
        let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
        return id.isEmpty ? nil : id
    }

    private static func uniqueIds(_ rawIds: [String]) -> [String] {
        var seen = Set<String>()
        return rawIds.compactMap { rawId in
            guard let id = normalizedId(rawId), seen.insert(id).inserted else { return nil }
            return id
        }
    }
}

/// The only object allowed to retain `PinLease` values. Its slots enumerate
/// every Core-owned residency lifetime that S3 will wire to App effects.
@MainActor
public final class GaryxThreadSummaryLeaseOwner {
    private enum Slot: Hashable {
        case page(String)
        case feed(String)
        case selectedThread
        case pickerResults(UInt64)
        case pickerSelectedTarget
        case widgetWrite(String)
        case composer(String)
        case botEntries(String)
    }

    public let cache: GaryxThreadSummaryCache
    private var leases: [Slot: GaryxThreadSummaryCache.PinLease] = [:]

    public init(cache: GaryxThreadSummaryCache) {
        self.cache = cache
    }

    public var activeLeaseCount: Int { leases.count }

    public func replacePage(
        ownerId: String,
        threadIds: [String],
        summaries: [GaryxThreadSummary]
    ) {
        replace(.page(ownerId), threadIds: threadIds, summaries: summaries)
    }

    public func removePage(ownerId: String) {
        release(.page(ownerId))
    }

    public func replaceFeed(
        ownerId: String,
        threadIds: [String],
        summaries: [GaryxThreadSummary]
    ) {
        replace(.feed(ownerId), threadIds: threadIds, summaries: summaries)
    }

    public func evictFeed(ownerId: String) {
        release(.feed(ownerId))
    }

    public func resetFeeds() {
        releaseSlots { if case .feed = $0 { return true }; return false }
    }

    /// Selected/open thread has an independent slot. Acquiring the new value
    /// before releasing the old makes thread-to-thread and thread-to-draft
    /// changes one atomic swap from the cache's point of view.
    public func swapSelectedThread(_ summary: GaryxThreadSummary?) {
        replace(
            .selectedThread,
            threadIds: summary.map { [$0.id] } ?? [],
            summaries: summary.map { [$0] } ?? []
        )
    }

    /// `q` identity is the trimmed original string plus this monotonic
    /// instance id. Replacing it releases every prior result-page lease in the
    /// same synchronous transaction after the new result lease is acquired.
    public func replacePickerQuery(
        instanceId: UInt64,
        threadIds: [String],
        summaries: [GaryxThreadSummary]
    ) {
        let slot = Slot.pickerResults(instanceId)
        let next = cache.makeLease(threadIds: threadIds, writing: summaries)
        releaseSlots { if case .pickerResults = $0 { return true }; return false }
        leases[slot] = next
    }

    public func closePicker() {
        releaseSlots {
            switch $0 {
            case .pickerResults, .pickerSelectedTarget: return true
            default: return false
            }
        }
    }

    public func swapPickerSelectedTarget(_ summary: GaryxThreadSummary?) {
        replace(
            .pickerSelectedTarget,
            threadIds: summary.map { [$0.id] } ?? [],
            summaries: summary.map { [$0] } ?? []
        )
    }

    public func beginWidgetWrite(
        token: String,
        threadIds: [String],
        summaries: [GaryxThreadSummary]
    ) {
        replace(.widgetWrite(token), threadIds: threadIds, summaries: summaries)
    }

    public func finishWidgetWrite(token: String) { release(.widgetWrite(token)) }
    public func cancelWidgetWrite(token: String) { release(.widgetWrite(token)) }
    public func skipWidgetWrite(token: String) { release(.widgetWrite(token)) }

    public func replaceComposerReferences(
        ownerId: String,
        threadIds: [String],
        summaries: [GaryxThreadSummary]
    ) {
        replace(.composer(ownerId), threadIds: threadIds, summaries: summaries)
    }

    public func settleComposer(ownerId: String) { release(.composer(ownerId)) }
    public func cancelComposer(ownerId: String) { release(.composer(ownerId)) }
    public func removeComposer(ownerId: String) { release(.composer(ownerId)) }

    public func replaceBotEntries(
        groupId: String,
        threadIds: [String],
        summaries: [GaryxThreadSummary]
    ) {
        replace(.botEntries(groupId), threadIds: threadIds, summaries: summaries)
    }

    public func removeBotEntries(groupId: String) { release(.botEntries(groupId)) }

    /// Gateway runtime-epoch reset releases every source before clearing the
    /// truth cache. No old lease can later resurrect or mutate the new scope.
    public func resetGatewayScope() {
        leases.removeAll(keepingCapacity: false)
        cache.removeAll()
    }

    private func replace(
        _ slot: Slot,
        threadIds: [String],
        summaries: [GaryxThreadSummary]
    ) {
        let next = cache.makeLease(threadIds: threadIds, writing: summaries)
        leases[slot] = next
    }

    private func release(_ slot: Slot) {
        leases[slot] = nil
    }

    private func releaseSlots(where predicate: (Slot) -> Bool) {
        for slot in leases.keys.filter(predicate) {
            leases[slot] = nil
        }
    }
}
