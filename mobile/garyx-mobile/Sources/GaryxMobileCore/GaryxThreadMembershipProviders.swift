import Combine
import Foundation

public enum GaryxThreadListProviderKind: Equatable, Hashable, Sendable {
    case recent(GaryxRecentThreadFilter)
    case workspace(path: String)
    case botConversations(groupId: String)
    case automationThreads(automationId: String)
    case favorites
    case picker
}

public struct GaryxThreadListProviderIdentity: Equatable, Hashable, Sendable {
    public var kind: GaryxThreadListProviderKind
    /// Trimmed original query. Canonical matching and cursor digesting remain
    /// wholly server-owned.
    public var query: String?
    public var instanceId: UInt64

    public init(
        kind: GaryxThreadListProviderKind,
        query: String? = nil,
        instanceId: UInt64
    ) {
        self.kind = kind
        let trimmed = query?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        self.query = trimmed.isEmpty ? nil : trimmed
        self.instanceId = instanceId
    }
}

public struct GaryxThreadListMembershipSnapshot: Equatable, Sendable {
    public var identity: GaryxThreadListProviderIdentity
    public var orderedThreadIds: [String]
    public var isPrimed: Bool
    public var isRefreshing: Bool
    public var headFailure: Bool
    public var footerState: GaryxHomeLoadMoreFooterState

    public init(
        identity: GaryxThreadListProviderIdentity,
        orderedThreadIds: [String] = [],
        isPrimed: Bool = false,
        isRefreshing: Bool = false,
        headFailure: Bool = false,
        footerState: GaryxHomeLoadMoreFooterState = .hidden
    ) {
        self.identity = identity
        self.orderedThreadIds = Self.uniqueIds(orderedThreadIds)
        self.isPrimed = isPrimed
        self.isRefreshing = isRefreshing
        self.headFailure = headFailure
        self.footerState = footerState
    }

    private static func uniqueIds(_ rawIds: [String]) -> [String] {
        var seen = Set<String>()
        return rawIds.compactMap { rawId in
            let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, seen.insert(id).inserted else { return nil }
            return id
        }
    }
}

/// A provider commit carries summaries only as a write-through payload. The
/// durable provider state and every emitted membership snapshot retain ids,
/// never leases or a second summary dictionary.
public struct GaryxThreadListMembershipCommit: Equatable, Sendable {
    public var snapshot: GaryxThreadListMembershipSnapshot
    public var summaryWrites: [GaryxThreadSummary]

    public init(
        snapshot: GaryxThreadListMembershipSnapshot,
        summaryWrites: [GaryxThreadSummary] = []
    ) {
        self.snapshot = snapshot
        self.summaryWrites = summaryWrites
    }
}

public protocol GaryxThreadListMembershipProvider: Sendable {
    @MainActor var identity: GaryxThreadListProviderIdentity { get }
    @MainActor var snapshot: GaryxThreadListMembershipSnapshot { get }
}

public enum GaryxThreadListProviderCompletion: Equatable, Sendable {
    case accepted(GaryxThreadListMembershipCommit)
    case rejectedStaleInstance
    case replacementRequired
}

// MARK: - Recent (zero-change wrapper)

public struct GaryxRecentThreadMembershipProvider: GaryxThreadListMembershipProvider,
    Equatable, Sendable {
    public private(set) var feeds: GaryxRecentThreadFeeds
    public let filter: GaryxRecentThreadFilter
    public let instanceId: UInt64

    public init(
        filter: GaryxRecentThreadFilter,
        pageLimit: Int = 30,
        overlap: Int = 5,
        instanceId: UInt64 = 1
    ) {
        self.filter = filter
        self.instanceId = instanceId
        feeds = GaryxRecentThreadFeeds(
            pageLimit: pageLimit,
            overlap: overlap,
            selectedFilter: filter
        )
    }

    public nonisolated var identity: GaryxThreadListProviderIdentity {
        GaryxThreadListProviderIdentity(kind: .recent(filter), instanceId: instanceId)
    }

    public nonisolated var snapshot: GaryxThreadListMembershipSnapshot {
        let feed = feeds.feed(for: filter)
        return GaryxThreadListMembershipSnapshot(
            identity: identity,
            orderedThreadIds: feed?.orderedThreadIds ?? [],
            isPrimed: feed?.isPrimed ?? (filter == .favorites),
            isRefreshing: feed?.pager.isRefreshingHead ?? false,
            headFailure: feed?.headFailure ?? false,
            footerState: feed?.pager.footerState ?? .hidden
        )
    }

    public mutating func requestRefresh(
        gatewayScope: String,
        runtimeEpoch: UInt64,
        forceReplacement: Bool = false
    ) -> GaryxRecentThreadRefreshTicket? {
        feeds.requestRefresh(
            filter: filter,
            gatewayScope: gatewayScope,
            runtimeEpoch: runtimeEpoch,
            forceReplacement: forceReplacement
        )
    }

    public mutating func completeRefresh(
        _ ticket: GaryxRecentThreadRefreshTicket,
        bundle: GaryxRecentThreadRefreshBundle,
        summaryWrites: [GaryxThreadSummary]
    ) -> GaryxThreadListProviderCompletion {
        switch feeds.completeRefresh(ticket, bundle: bundle) {
        case .applied:
            return .accepted(
                GaryxThreadListMembershipCommit(
                    snapshot: snapshot,
                    summaryWrites: summaryWrites
                )
            )
        case .forceReplacement:
            return .replacementRequired
        case .abandonedStaleEpoch, .abandonedLocalMutation:
            return .rejectedStaleInstance
        }
    }
}

// MARK: - Workspace / unscoped picker

public enum GaryxThreadSummaryProviderScope: Equatable, Hashable, Sendable {
    case workspace(path: String)
    case unscopedPicker(query: String?)

    public var workspacePath: String? {
        guard case .workspace(let path) = self else { return nil }
        return path
    }

    public var originalTrimmedQuery: String? {
        guard case .unscopedPicker(let query) = self else { return nil }
        let trimmed = query?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }
}

public struct GaryxThreadSummaryRefreshTicket: Equatable, Sendable {
    public var instanceId: UInt64
    public var pagerTicket: GaryxThreadListRefreshTicket
    public var workspacePath: String?
    public var query: String?
}

public struct GaryxThreadSummaryLoadMoreTicket: Equatable, Sendable {
    public var instanceId: UInt64
    public var pagerTicket: GaryxThreadListLoadMoreTicket
    public var workspacePath: String?
    public var query: String?
    public var cursor: String
}

public struct GaryxThreadSummaryMembershipProvider: GaryxThreadListMembershipProvider,
    Equatable, Sendable {
    public private(set) var scope: GaryxThreadSummaryProviderScope
    public private(set) var instanceId: UInt64
    public private(set) var orderedThreadIds: [String]
    public private(set) var pager: GaryxHomeThreadListPager
    public private(set) var nextCursor: String?
    public private(set) var storeIncarnationId: String?
    public private(set) var serverBootId: String?
    public private(set) var isPrimed: Bool
    public private(set) var headFailure: Bool

    public init(
        scope: GaryxThreadSummaryProviderScope,
        pageLimit: Int = 30,
        overlap: Int = 5,
        instanceId: UInt64 = 1
    ) {
        self.scope = Self.normalized(scope)
        self.instanceId = max(1, instanceId)
        orderedThreadIds = []
        pager = GaryxHomeThreadListPager(pageLimit: pageLimit, overlap: overlap)
        nextCursor = nil
        storeIncarnationId = nil
        serverBootId = nil
        isPrimed = false
        headFailure = false
    }

    public nonisolated var identity: GaryxThreadListProviderIdentity {
        switch scope {
        case .workspace(let path):
            return GaryxThreadListProviderIdentity(
                kind: .workspace(path: path),
                instanceId: instanceId
            )
        case .unscopedPicker:
            return GaryxThreadListProviderIdentity(
                kind: .picker,
                query: scope.originalTrimmedQuery,
                instanceId: instanceId
            )
        }
    }

    public nonisolated var snapshot: GaryxThreadListMembershipSnapshot {
        GaryxThreadListMembershipSnapshot(
            identity: identity,
            orderedThreadIds: orderedThreadIds,
            isPrimed: isPrimed,
            isRefreshing: pager.isRefreshingHead,
            headFailure: headFailure,
            footerState: pager.footerState
        )
    }

    public mutating func requestRefresh() -> GaryxThreadSummaryRefreshTicket? {
        guard !pager.isLoadingMore, let ticket = pager.requestRefresh() else { return nil }
        headFailure = false
        return GaryxThreadSummaryRefreshTicket(
            instanceId: instanceId,
            pagerTicket: ticket,
            workspacePath: scope.workspacePath,
            query: scope.originalTrimmedQuery
        )
    }

    public mutating func requestLoadMore(
        trigger: GaryxThreadListLoadMoreTrigger
    ) -> GaryxThreadSummaryLoadMoreTicket? {
        guard !pager.isRefreshingHead,
              let nextCursor,
              let ticket = pager.requestLoadMore(trigger: trigger) else { return nil }
        return GaryxThreadSummaryLoadMoreTicket(
            instanceId: instanceId,
            pagerTicket: ticket,
            workspacePath: scope.workspacePath,
            query: scope.originalTrimmedQuery,
            cursor: nextCursor
        )
    }

    public mutating func retryLoadMore() -> GaryxThreadSummaryLoadMoreTicket? {
        guard !pager.isRefreshingHead,
              let nextCursor,
              let ticket = pager.retryLoadMore() else { return nil }
        return GaryxThreadSummaryLoadMoreTicket(
            instanceId: instanceId,
            pagerTicket: ticket,
            workspacePath: scope.workspacePath,
            query: scope.originalTrimmedQuery,
            cursor: nextCursor
        )
    }

    public mutating func completeRefresh(
        _ ticket: GaryxThreadSummaryRefreshTicket,
        page: GaryxThreadSummariesPage
    ) -> GaryxThreadListProviderCompletion {
        guard owns(ticket) else { return .rejectedStaleInstance }
        guard identityAccepts(page) else {
            coldReset(advanceInstance: true)
            return .replacementRequired
        }
        let application = pager.completeRefresh(
            ticket.pagerTicket,
            pageOffset: 0,
            pageCount: page.threads.count,
            hasMore: page.hasMore
        )
        guard case .apply(let merge) = application else { return .rejectedStaleInstance }
        let ids = Self.uniqueIds(page.threads.map(\.id))
        switch merge {
        case .replaceHead:
            orderedThreadIds = ids
        case .mergeBeyondHead:
            orderedThreadIds = GaryxThreadListPageMerge.mergeHead(
                pageIds: ids,
                existingIds: orderedThreadIds
            )
        }
        nextCursor = page.nextCursor
        storeIncarnationId = page.storeIncarnationId
        serverBootId = page.serverBootId
        isPrimed = true
        headFailure = false
        return .accepted(
            GaryxThreadListMembershipCommit(snapshot: snapshot, summaryWrites: page.threads)
        )
    }

    public mutating func failRefresh(_ ticket: GaryxThreadSummaryRefreshTicket) {
        guard owns(ticket) else { return }
        pager.failRefresh(ticket.pagerTicket)
        headFailure = true
    }

    public mutating func completeLoadMore(
        _ ticket: GaryxThreadSummaryLoadMoreTicket,
        page: GaryxThreadSummariesPage
    ) -> GaryxThreadListProviderCompletion {
        guard owns(ticket) else { return .rejectedStaleInstance }
        guard identityAccepts(page) else {
            coldReset(advanceInstance: true)
            return .replacementRequired
        }
        guard pager.completeLoadMore(
            ticket.pagerTicket,
            pageOffset: pager.nextOffset,
            pageCount: page.threads.count,
            hasMore: page.hasMore
        ) == .apply else { return .rejectedStaleInstance }
        orderedThreadIds = GaryxThreadListPageMerge.appendPage(
            pageIds: Self.uniqueIds(page.threads.map(\.id)),
            existingIds: orderedThreadIds
        )
        nextCursor = page.nextCursor
        return .accepted(
            GaryxThreadListMembershipCommit(snapshot: snapshot, summaryWrites: page.threads)
        )
    }

    /// Returns true when an actual query change created a fresh instance.
    /// The caller cancels transport work and atomically swaps the old result
    /// lease when it observes true.
    @discardableResult
    public mutating func replacePickerQuery(_ rawQuery: String?) -> Bool {
        guard case .unscopedPicker = scope else { return false }
        let next = GaryxThreadSummaryProviderScope.unscopedPicker(query: rawQuery)
        let normalized = Self.normalized(next)
        guard normalized != scope else { return false }
        scope = normalized
        coldReset(advanceInstance: true)
        return true
    }

    public mutating func resetGatewayScope() {
        coldReset(advanceInstance: true)
    }

    public mutating func apply(
        _ authority: GaryxThreadMutationMembershipAuthority,
        summary: GaryxThreadSummary? = nil
    ) {
        switch authority {
        case .unchanged:
            break
        case .remove(let rawThreadId):
            let threadId = rawThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !threadId.isEmpty else { return }
            pager.noteLocalMutation()
            orderedThreadIds.removeAll { $0 == threadId }
        case .upsertAtHead(let rawThreadId):
            let threadId = rawThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !threadId.isEmpty else { return }
            if case .workspace(let path) = scope,
               summary?.workspacePath?.trimmingCharacters(in: .whitespacesAndNewlines) != path {
                return
            }
            pager.noteLocalMutation()
            orderedThreadIds.removeAll { $0 == threadId }
            orderedThreadIds.insert(threadId, at: 0)
        case .replace(let ids, _):
            pager.noteLocalMutation()
            orderedThreadIds = Self.uniqueIds(ids)
        }
    }

    private func owns(_ ticket: GaryxThreadSummaryRefreshTicket) -> Bool {
        ticket.instanceId == instanceId
            && ticket.workspacePath == scope.workspacePath
            && ticket.query == scope.originalTrimmedQuery
    }

    private func owns(_ ticket: GaryxThreadSummaryLoadMoreTicket) -> Bool {
        ticket.instanceId == instanceId
            && ticket.workspacePath == scope.workspacePath
            && ticket.query == scope.originalTrimmedQuery
    }

    private func identityAccepts(_ page: GaryxThreadSummariesPage) -> Bool {
        (storeIncarnationId == nil || storeIncarnationId == page.storeIncarnationId)
            && (serverBootId == nil || serverBootId == page.serverBootId)
    }

    private mutating func coldReset(advanceInstance: Bool) {
        if advanceInstance { instanceId &+= 1 }
        pager.reset()
        orderedThreadIds = []
        nextCursor = nil
        storeIncarnationId = nil
        serverBootId = nil
        isPrimed = false
        headFailure = false
    }

    public mutating func failLoadMore(_ ticket: GaryxThreadSummaryLoadMoreTicket) {
        guard owns(ticket) else { return }
        pager.failLoadMore(ticket.pagerTicket)
    }

    private static func normalized(
        _ scope: GaryxThreadSummaryProviderScope
    ) -> GaryxThreadSummaryProviderScope {
        switch scope {
        case .workspace(let path):
            return .workspace(path: path)
        case .unscopedPicker(let query):
            let trimmed = query?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            return .unscopedPicker(query: trimmed.isEmpty ? nil : trimmed)
        }
    }

    private static func uniqueIds(_ rawIds: [String]) -> [String] {
        var seen = Set<String>()
        return rawIds.compactMap { rawId in
            let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, seen.insert(id).inserted else { return nil }
            return id
        }
    }
}

/// Main-actor picker owner that binds the copyable provider state to its
/// noncopyable lease sidecar. Query replacement is one synchronous boundary:
/// old transport ownership is revoked, instance generation advances, and old
/// result pins are released before the empty replacement snapshot publishes.
@MainActor
public final class GaryxThreadPickerMembershipOwner: ObservableObject {
    @Published public private(set) var snapshot: GaryxThreadListMembershipSnapshot
    @Published public private(set) var availability: GaryxThreadListAvailability = .ready
    public private(set) var provider: GaryxThreadSummaryMembershipProvider
    public private(set) var publishCount = 0

    public let cache: GaryxThreadSummaryCache
    public let leaseOwner: GaryxThreadSummaryLeaseOwner
    public var onCancelInstance: ((UInt64) -> Void)?

    public init(
        query: String? = nil,
        cache: GaryxThreadSummaryCache,
        leaseOwner: GaryxThreadSummaryLeaseOwner,
        instanceId: UInt64 = 1
    ) {
        self.cache = cache
        self.leaseOwner = leaseOwner
        provider = GaryxThreadSummaryMembershipProvider(
            scope: .unscopedPicker(query: query),
            instanceId: instanceId
        )
        snapshot = provider.snapshot
    }

    public var identity: GaryxThreadListProviderIdentity { provider.identity }

    public func requestRefresh() -> GaryxThreadSummaryRefreshTicket? {
        provider.requestRefresh()
    }

    public func requestLoadMore(
        trigger: GaryxThreadListLoadMoreTrigger
    ) -> GaryxThreadSummaryLoadMoreTicket? {
        provider.requestLoadMore(trigger: trigger)
    }

    public func retryLoadMore() -> GaryxThreadSummaryLoadMoreTicket? {
        provider.retryLoadMore()
    }

    @discardableResult
    public func completeRefresh(
        _ ticket: GaryxThreadSummaryRefreshTicket,
        page: GaryxThreadSummariesPage
    ) -> GaryxThreadListProviderCompletion {
        let previousInstanceId = provider.instanceId
        return commit(
            provider.completeRefresh(ticket, page: page),
            previousInstanceId: previousInstanceId
        )
    }

    @discardableResult
    public func completeLoadMore(
        _ ticket: GaryxThreadSummaryLoadMoreTicket,
        page: GaryxThreadSummariesPage
    ) -> GaryxThreadListProviderCompletion {
        let previousInstanceId = provider.instanceId
        return commit(
            provider.completeLoadMore(ticket, page: page),
            previousInstanceId: previousInstanceId
        )
    }

    public func failRefresh(_ ticket: GaryxThreadSummaryRefreshTicket) {
        provider.failRefresh(ticket)
        publish(provider.snapshot)
    }

    public func failLoadMore(_ ticket: GaryxThreadSummaryLoadMoreTicket) {
        provider.failLoadMore(ticket)
        publish(provider.snapshot)
    }

    public func setAvailability(_ availability: GaryxThreadListAvailability) {
        self.availability = availability
    }

    /// Old-gateway-only bounded fallback. The provider identity remains the
    /// current query instance so any late server page is still fenced.
    public func presentUnsupportedFallback(_ summaries: [GaryxThreadSummary]) {
        let rows = GaryxLegacyThreadPickerFallback.rows(
            recentRows: summaries,
            rawQuery: identity.query
        )
        leaseOwner.replacePickerQuery(
            instanceId: identity.instanceId,
            threadIds: rows.map(\.id),
            summaries: rows
        )
        publish(
            GaryxThreadListMembershipSnapshot(
                identity: identity,
                orderedThreadIds: rows.map(\.id),
                isPrimed: true
            )
        )
        availability = .unsupportedGateway
    }

    @discardableResult
    public func replaceQuery(_ rawQuery: String?) -> Bool {
        let previousInstanceId = provider.instanceId
        guard provider.replacePickerQuery(rawQuery) else { return false }
        onCancelInstance?(previousInstanceId)
        leaseOwner.replacePickerQuery(
            instanceId: provider.instanceId,
            threadIds: [],
            summaries: []
        )
        publish(provider.snapshot)
        availability = .ready
        return true
    }

    public func swapSelectedTarget(_ summary: GaryxThreadSummary?) {
        leaseOwner.swapPickerSelectedTarget(summary)
    }

    public func close() {
        let previousInstanceId = provider.instanceId
        provider.resetGatewayScope()
        onCancelInstance?(previousInstanceId)
        leaseOwner.closePicker()
        publish(provider.snapshot)
        availability = .ready
    }

    private func commit(
        _ completion: GaryxThreadListProviderCompletion,
        previousInstanceId: UInt64
    ) -> GaryxThreadListProviderCompletion {
        switch completion {
        case .accepted(let commit):
            leaseOwner.replacePickerQuery(
                instanceId: commit.snapshot.identity.instanceId,
                threadIds: commit.snapshot.orderedThreadIds,
                summaries: commit.summaryWrites
            )
            publish(commit.snapshot)
            availability = .ready
        case .replacementRequired:
            onCancelInstance?(previousInstanceId)
            leaseOwner.replacePickerQuery(
                instanceId: provider.instanceId,
                threadIds: [],
                summaries: []
            )
            publish(provider.snapshot)
        case .rejectedStaleInstance:
            break
        }
        return completion
    }

    private func publish(_ next: GaryxThreadListMembershipSnapshot) {
        guard next != snapshot else { return }
        snapshot = next
        publishCount += 1
    }
}

// MARK: - Bot conversations

public struct GaryxBotConversationMembershipEntry: Equatable, Sendable {
    public var threadId: String
    public var endpointKey: String
    public var openable: Bool
    public var canArchiveEndpoint: Bool

    public init(
        threadId: String,
        endpointKey: String,
        openable: Bool = true,
        canArchiveEndpoint: Bool = true
    ) {
        self.threadId = threadId
        self.endpointKey = endpointKey
        self.openable = openable
        self.canArchiveEndpoint = canArchiveEndpoint
    }
}

public struct GaryxBotConversationHydrationTicket: Equatable, Sendable {
    public var groupId: String
    public var instanceId: UInt64
    public var threadId: String
}

public struct GaryxBotConversationMembershipUpdate: Equatable, Sendable {
    public var commit: GaryxThreadListMembershipCommit
    public var hydrationTickets: [GaryxBotConversationHydrationTicket]
    public var cancelledInstanceId: UInt64?
}

public struct GaryxBotConversationMembershipProvider: GaryxThreadListMembershipProvider,
    Equatable, Sendable {
    public let groupId: String
    public private(set) var instanceId: UInt64
    public private(set) var entries: [GaryxBotConversationMembershipEntry]

    public init(groupId: String, instanceId: UInt64 = 1) {
        self.groupId = groupId
        self.instanceId = max(1, instanceId)
        entries = []
    }

    public nonisolated var identity: GaryxThreadListProviderIdentity {
        GaryxThreadListProviderIdentity(
            kind: .botConversations(groupId: groupId),
            instanceId: instanceId
        )
    }

    public nonisolated var snapshot: GaryxThreadListMembershipSnapshot {
        GaryxThreadListMembershipSnapshot(
            identity: identity,
            orderedThreadIds: entries.map(\.threadId),
            isPrimed: true
        )
    }

    public mutating func replaceEntries(
        _ nextEntries: [GaryxBotConversationMembershipEntry],
        availableSummaryIds: Set<String>
    ) -> GaryxBotConversationMembershipUpdate {
        let cancelled = instanceId
        instanceId &+= 1
        var seen = Set<String>()
        entries = nextEntries.compactMap { entry in
            let id = entry.threadId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, entry.openable, seen.insert(id).inserted else { return nil }
            var normalized = entry
            normalized.threadId = id
            return normalized
        }
        let missing = entries.map(\.threadId).filter { !availableSummaryIds.contains($0) }
        return GaryxBotConversationMembershipUpdate(
            commit: GaryxThreadListMembershipCommit(snapshot: snapshot),
            hydrationTickets: missing.map {
                GaryxBotConversationHydrationTicket(
                    groupId: groupId,
                    instanceId: instanceId,
                    threadId: $0
                )
            },
            cancelledInstanceId: cancelled
        )
    }

    public func completeHydration(
        _ ticket: GaryxBotConversationHydrationTicket,
        summary: GaryxThreadSummary
    ) -> GaryxThreadListProviderCompletion {
        guard ticket.groupId == groupId,
              ticket.instanceId == instanceId,
              ticket.threadId == summary.id,
              entries.contains(where: { $0.threadId == ticket.threadId }) else {
            return .rejectedStaleInstance
        }
        return .accepted(
            GaryxThreadListMembershipCommit(snapshot: snapshot, summaryWrites: [summary])
        )
    }

    public mutating func apply(_ authority: GaryxThreadMutationMembershipAuthority) {
        switch authority {
        case .unchanged:
            break
        case .remove(let rawThreadId):
            let threadId = rawThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
            entries.removeAll { $0.threadId == threadId }
        case .upsertAtHead, .replace:
            // Bot membership is endpoint-owned and is replaced from the
            // configured console/endpoints projection, never inferred from a
            // generic thread insertion.
            break
        }
    }
}

// MARK: - Automation triggered threads

public struct GaryxAutomationThreadRefreshTicket: Equatable, Sendable {
    public var automationId: String
    public var instanceId: UInt64
    public var pagerTicket: GaryxThreadListRefreshTicket
}

public struct GaryxAutomationThreadLoadMoreTicket: Equatable, Sendable {
    public var automationId: String
    public var instanceId: UInt64
    public var pagerTicket: GaryxThreadListLoadMoreTicket
}

public struct GaryxAutomationThreadMembershipProvider: GaryxThreadListMembershipProvider,
    Equatable, Sendable {
    public let automationId: String
    public private(set) var instanceId: UInt64
    public private(set) var orderedThreadIds: [String]
    public private(set) var pager: GaryxHomeThreadListPager
    public private(set) var isPrimed: Bool
    public private(set) var headFailure: Bool

    public init(
        automationId: String,
        pageLimit: Int = 50,
        overlap: Int = 5,
        instanceId: UInt64 = 1
    ) {
        self.automationId = automationId
        self.instanceId = max(1, instanceId)
        orderedThreadIds = []
        pager = GaryxHomeThreadListPager(pageLimit: pageLimit, overlap: overlap)
        isPrimed = false
        headFailure = false
    }

    public nonisolated var identity: GaryxThreadListProviderIdentity {
        GaryxThreadListProviderIdentity(
            kind: .automationThreads(automationId: automationId),
            instanceId: instanceId
        )
    }

    public nonisolated var snapshot: GaryxThreadListMembershipSnapshot {
        GaryxThreadListMembershipSnapshot(
            identity: identity,
            orderedThreadIds: orderedThreadIds,
            isPrimed: isPrimed,
            isRefreshing: pager.isRefreshingHead,
            headFailure: headFailure,
            footerState: pager.footerState
        )
    }

    public mutating func requestRefresh() -> GaryxAutomationThreadRefreshTicket? {
        guard !pager.isLoadingMore, let ticket = pager.requestRefresh() else { return nil }
        headFailure = false
        return GaryxAutomationThreadRefreshTicket(
            automationId: automationId,
            instanceId: instanceId,
            pagerTicket: ticket
        )
    }

    public mutating func requestLoadMore(
        trigger: GaryxThreadListLoadMoreTrigger
    ) -> GaryxAutomationThreadLoadMoreTicket? {
        guard !pager.isRefreshingHead,
              let ticket = pager.requestLoadMore(trigger: trigger) else { return nil }
        return GaryxAutomationThreadLoadMoreTicket(
            automationId: automationId,
            instanceId: instanceId,
            pagerTicket: ticket
        )
    }

    public mutating func retryLoadMore() -> GaryxAutomationThreadLoadMoreTicket? {
        guard !pager.isRefreshingHead,
              let ticket = pager.retryLoadMore() else { return nil }
        return GaryxAutomationThreadLoadMoreTicket(
            automationId: automationId,
            instanceId: instanceId,
            pagerTicket: ticket
        )
    }

    public mutating func completeRefresh(
        _ ticket: GaryxAutomationThreadRefreshTicket,
        page: GaryxAutomationThreadsPage
    ) -> GaryxThreadListProviderCompletion {
        guard owns(ticket) else { return .rejectedStaleInstance }
        guard page.automationId == automationId else {
            resetGatewayScope()
            return .replacementRequired
        }
        let result = pager.completeRefresh(
            ticket.pagerTicket,
            pageOffset: page.offset,
            pageCount: page.count,
            hasMore: page.hasMore
        )
        guard case .apply(let application) = result else { return .rejectedStaleInstance }
        let ids = Self.ids(page)
        switch application {
        case .replaceHead: orderedThreadIds = ids
        case .mergeBeyondHead:
            orderedThreadIds = GaryxThreadListPageMerge.mergeHead(
                pageIds: ids,
                existingIds: orderedThreadIds
            )
        }
        isPrimed = true
        headFailure = false
        return commit(page)
    }

    public mutating func completeLoadMore(
        _ ticket: GaryxAutomationThreadLoadMoreTicket,
        page: GaryxAutomationThreadsPage
    ) -> GaryxThreadListProviderCompletion {
        guard owns(ticket) else { return .rejectedStaleInstance }
        guard page.automationId == automationId else {
            resetGatewayScope()
            return .replacementRequired
        }
        guard pager.completeLoadMore(
            ticket.pagerTicket,
            pageOffset: page.offset,
            pageCount: page.count,
            hasMore: page.hasMore
        ) == .apply else { return .rejectedStaleInstance }
        orderedThreadIds = GaryxThreadListPageMerge.appendPage(
            pageIds: Self.ids(page),
            existingIds: orderedThreadIds
        )
        return commit(page)
    }

    public mutating func resetGatewayScope() {
        instanceId &+= 1
        pager.reset()
        orderedThreadIds = []
        isPrimed = false
        headFailure = false
    }

    public mutating func failRefresh(_ ticket: GaryxAutomationThreadRefreshTicket) {
        guard owns(ticket) else { return }
        pager.failRefresh(ticket.pagerTicket)
        headFailure = true
    }

    public mutating func failLoadMore(_ ticket: GaryxAutomationThreadLoadMoreTicket) {
        guard owns(ticket) else { return }
        pager.failLoadMore(ticket.pagerTicket)
    }

    public mutating func apply(_ authority: GaryxThreadMutationMembershipAuthority) {
        switch authority {
        case .unchanged, .upsertAtHead:
            break
        case .remove(let rawThreadId):
            let threadId = rawThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !threadId.isEmpty else { return }
            pager.noteLocalMutation()
            orderedThreadIds.removeAll { $0 == threadId }
        case .replace(let ids, _):
            pager.noteLocalMutation()
            orderedThreadIds = Self.uniqueIds(ids)
        }
    }

    private func owns(_ ticket: GaryxAutomationThreadRefreshTicket) -> Bool {
        ticket.automationId == automationId && ticket.instanceId == instanceId
    }

    private func owns(_ ticket: GaryxAutomationThreadLoadMoreTicket) -> Bool {
        ticket.automationId == automationId && ticket.instanceId == instanceId
    }

    private func commit(_ page: GaryxAutomationThreadsPage) -> GaryxThreadListProviderCompletion {
        .accepted(
            GaryxThreadListMembershipCommit(
                snapshot: snapshot,
                summaryWrites: page.items.compactMap(\.thread)
            )
        )
    }

    private static func ids(_ page: GaryxAutomationThreadsPage) -> [String] {
        var seen = Set<String>()
        return page.items.compactMap { item in
            let id = item.threadId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, seen.insert(id).inserted else { return nil }
            return id
        }
    }

    private static func uniqueIds(_ rawIds: [String]) -> [String] {
        var seen = Set<String>()
        return rawIds.compactMap { rawId in
            let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, seen.insert(id).inserted else { return nil }
            return id
        }
    }
}

// MARK: - Instance registry / LRU

public enum GaryxThreadFeedRegistryKey: Equatable, Hashable, Sendable {
    case recentAll
    case recentChats
    case favorites
    case workspace(String)
    case picker(String)
}

public struct GaryxThreadFeedRegistryTicket: Equatable, Hashable, Sendable {
    public var key: GaryxThreadFeedRegistryKey
    public var instanceId: UInt64
    public var requestId: UInt64
}

public struct GaryxThreadFeedActivation: Equatable, Sendable {
    public var key: GaryxThreadFeedRegistryKey
    public var instanceId: UInt64
    public var coldLoad: Bool
    public var evicted: [(key: GaryxThreadFeedRegistryKey, instanceId: UInt64)]

    public static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.key == rhs.key
            && lhs.instanceId == rhs.instanceId
            && lhs.coldLoad == rhs.coldLoad
            && lhs.evicted.elementsEqual(rhs.evicted, by: ==)
    }
}

public struct GaryxThreadFeedRegistry: Equatable, Sendable {
    private struct Entry: Equatable, Sendable {
        var instanceId: UInt64
        var lastAccess: UInt64
    }

    public let workspaceCapacity: Int
    private var entries: [GaryxThreadFeedRegistryKey: Entry]
    private var clock: UInt64
    private var nextInstanceId: UInt64
    private var nextRequestId: UInt64

    public init(workspaceCapacity: Int = 4) {
        self.workspaceCapacity = max(0, workspaceCapacity)
        clock = 3
        nextInstanceId = 4
        nextRequestId = 1
        entries = [
            .recentAll: Entry(instanceId: 1, lastAccess: 1),
            .recentChats: Entry(instanceId: 2, lastAccess: 2),
            .favorites: Entry(instanceId: 3, lastAccess: 3),
        ]
    }

    public mutating func activateWorkspace(_ path: String) -> GaryxThreadFeedActivation {
        activate(.workspace(path))
    }

    public mutating func activate(_ key: GaryxThreadFeedRegistryKey) -> GaryxThreadFeedActivation {
        clock &+= 1
        if var entry = entries[key] {
            entry.lastAccess = clock
            entries[key] = entry
            return GaryxThreadFeedActivation(
                key: key,
                instanceId: entry.instanceId,
                coldLoad: false,
                evicted: []
            )
        }
        let instanceId = nextInstanceId
        nextInstanceId &+= 1
        entries[key] = Entry(instanceId: instanceId, lastAccess: clock)
        var evicted: [(key: GaryxThreadFeedRegistryKey, instanceId: UInt64)] = []
        if case .workspace = key {
            let workspaces = entries.compactMap { candidate -> (GaryxThreadFeedRegistryKey, Entry)? in
                guard case .workspace = candidate.key else { return nil }
                return (candidate.key, candidate.value)
            }.sorted {
                if $0.1.lastAccess != $1.1.lastAccess {
                    return $0.1.lastAccess < $1.1.lastAccess
                }
                return String(describing: $0.0) < String(describing: $1.0)
            }
            for (evictedKey, entry) in workspaces.prefix(max(0, workspaces.count - workspaceCapacity)) {
                entries[evictedKey] = nil
                evicted.append((evictedKey, entry.instanceId))
            }
        }
        return GaryxThreadFeedActivation(
            key: key,
            instanceId: instanceId,
            coldLoad: true,
            evicted: evicted
        )
    }

    public mutating func replacePicker(
        ownerId: String,
        query _: String?
    ) -> GaryxThreadFeedActivation {
        let key = GaryxThreadFeedRegistryKey.picker(ownerId)
        let evicted = entries.removeValue(forKey: key).map { [(key, $0.instanceId)] } ?? []
        let activation = activate(key)
        return GaryxThreadFeedActivation(
            key: key,
            instanceId: activation.instanceId,
            coldLoad: true,
            evicted: evicted
        )
    }

    public mutating func issueTicket(
        for key: GaryxThreadFeedRegistryKey
    ) -> GaryxThreadFeedRegistryTicket? {
        guard let entry = entries[key] else { return nil }
        defer { nextRequestId &+= 1 }
        return GaryxThreadFeedRegistryTicket(
            key: key,
            instanceId: entry.instanceId,
            requestId: nextRequestId
        )
    }

    public func accepts(_ ticket: GaryxThreadFeedRegistryTicket) -> Bool {
        entries[ticket.key]?.instanceId == ticket.instanceId
    }

    public mutating func resetGatewayScope() {
        entries.removeAll(keepingCapacity: true)
        for key in [
            GaryxThreadFeedRegistryKey.recentAll,
            .recentChats,
            .favorites,
        ] {
            clock &+= 1
            entries[key] = Entry(instanceId: nextInstanceId, lastAccess: clock)
            nextInstanceId &+= 1
        }
        nextRequestId &+= 1
    }
}

// MARK: - Shared presentation/action store

public struct GaryxThreadListPresentationSnapshot: Equatable, Sendable {
    public var identity: GaryxThreadListProviderIdentity?
    public var pinnedThreadIds: [String]
    public var orderedThreadIds: [String]
    public var rows: [GaryxThreadSummary]
    public var capabilitiesById: [String: GaryxThreadRowCapabilities]
    public var isPrimed: Bool
    public var isRefreshing: Bool
    public var headFailure: Bool
    public var footerState: GaryxHomeLoadMoreFooterState
    public var availability: GaryxThreadListAvailability
    public var selectedThreadId: String?
    public var pinnedStateThreadIds: Set<String>
    public var favoriteThreadIds: Set<String>
    public var activeRunThreadIds: Set<String>
    public var motionById: [String: GaryxThreadRowMotion]

    public static let empty = GaryxThreadListPresentationSnapshot(
        identity: nil,
        pinnedThreadIds: [],
        orderedThreadIds: [],
        rows: [],
        capabilitiesById: [:],
        isPrimed: false,
        isRefreshing: false,
        headFailure: false,
        footerState: .hidden,
        availability: .ready,
        selectedThreadId: nil,
        pinnedStateThreadIds: [],
        favoriteThreadIds: [],
        activeRunThreadIds: [],
        motionById: [:]
    )
}

/// Generalized Core store consumed by all provider kinds. The existing
/// `GaryxHomeThreadListStore` remains the Home compatibility specialization
/// until S3 moves its view bindings; both use the same provider commit and
/// action-capability contracts.
@MainActor
public final class GaryxThreadListStore: ObservableObject {
    @Published public private(set) var snapshot: GaryxThreadListPresentationSnapshot
    public private(set) var publishCount = 0

    public let cache: GaryxThreadSummaryCache
    public let leaseOwner: GaryxThreadSummaryLeaseOwner
    private let ownerId: String
    private var pinnedThreadIds: [String]

    public init(
        ownerId: String,
        cache: GaryxThreadSummaryCache,
        leaseOwner: GaryxThreadSummaryLeaseOwner,
        pinnedThreadIds: [String] = []
    ) {
        self.ownerId = ownerId
        self.cache = cache
        self.leaseOwner = leaseOwner
        self.pinnedThreadIds = Self.uniqueIds(pinnedThreadIds)
        snapshot = .empty
    }

    @discardableResult
    public func commit(
        _ commit: GaryxThreadListMembershipCommit,
        favoriteThreadIds: Set<String> = [],
        pinnedStateThreadIds: Set<String> = [],
        selectedThreadId: String? = nil,
        automationTargetThreadIds: Set<String> = [],
        activeRunThreadIds: Set<String> = [],
        pendingMutations: [GaryxThreadMutationID: GaryxThreadMutationPendingState] = [:],
        botEntries: [String: GaryxBotConversationMembershipEntry] = [:]
    ) -> Bool {
        let isRecentAll: Bool = {
            guard case .recent(.all) = commit.snapshot.identity.kind else { return false }
            return true
        }()
        let effectivePinnedIds = isRecentAll ? pinnedThreadIds : []
        leaseOwner.replaceFeed(
            ownerId: ownerId,
            threadIds: effectivePinnedIds + commit.snapshot.orderedThreadIds,
            summaries: commit.summaryWrites
        )
        let pinnedSet = Set(effectivePinnedIds)
        let ids = commit.snapshot.orderedThreadIds.filter { !pinnedSet.contains($0) }
        let resolvedIds = effectivePinnedIds + ids
        let rows = resolvedIds.compactMap(cache.summary(for:))
        let rowIds = Set(rows.map(\.id))
        let visiblePinned = effectivePinnedIds.filter(rowIds.contains)
        let visibleOrdered = ids.filter(rowIds.contains)
        var capabilities: [String: GaryxThreadRowCapabilities] = [:]
        for row in rows {
            let bot = botEntries[row.id]
            capabilities[row.id] = GaryxThreadRowCapabilityDeriver.capabilities(
                for: row,
                context: GaryxThreadRowCapabilityContext(
                    openable: bot?.openable ?? true,
                    isFavorite: favoriteThreadIds.contains(row.id),
                    automationTargetThreadIds: automationTargetThreadIds,
                    hasActiveRun: activeRunThreadIds.contains(row.id),
                    botEndpointRow: bot != nil,
                    botEndpointCanArchive: bot?.canArchiveEndpoint ?? true
                )
            )
        }
        var motionById: [String: GaryxThreadRowMotion] = [:]
        for pending in pendingMutations.values where pending.showsMotion {
            let threadId = pending.kind.threadId
            switch pending.kind {
            case .archive:
                motionById[threadId] = .archiving
            case .pin:
                if motionById[threadId] != .archiving {
                    motionById[threadId] = .pinning
                }
            case .insert, .rename, .runtime, .favoriteDownstream:
                break
            }
        }
        let next = GaryxThreadListPresentationSnapshot(
            identity: commit.snapshot.identity,
            pinnedThreadIds: visiblePinned,
            orderedThreadIds: visibleOrdered,
            rows: rows,
            capabilitiesById: capabilities,
            isPrimed: commit.snapshot.isPrimed,
            isRefreshing: commit.snapshot.isRefreshing,
            headFailure: commit.snapshot.headFailure,
            footerState: commit.snapshot.footerState,
            availability: snapshot.availability,
            selectedThreadId: selectedThreadId,
            pinnedStateThreadIds: pinnedStateThreadIds,
            favoriteThreadIds: favoriteThreadIds,
            activeRunThreadIds: activeRunThreadIds,
            motionById: motionById
        )
        guard next != snapshot else { return false }
        snapshot = next
        publishCount += 1
        return true
    }

    public func replacePinnedOrder(_ ids: [String]) {
        pinnedThreadIds = Self.uniqueIds(ids)
    }

    public func setAvailability(_ availability: GaryxThreadListAvailability) {
        guard snapshot.availability != availability else { return }
        snapshot.availability = availability
        publishCount += 1
    }

    public func resetGatewayScope() {
        leaseOwner.evictFeed(ownerId: ownerId)
        if snapshot != .empty {
            snapshot = .empty
            publishCount += 1
        }
    }

    private static func uniqueIds(_ rawIds: [String]) -> [String] {
        var seen = Set<String>()
        return rawIds.compactMap { rawId in
            let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, seen.insert(id).inserted else { return nil }
            return id
        }
    }
}
