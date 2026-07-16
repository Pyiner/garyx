import Foundation

public enum GaryxRecentThreadFilter: String, CaseIterable, Equatable, Hashable, Sendable {
    case all
    case nonTask

    /// The product surface exposed by the Home filter menu. Keep this
    /// explicit so future internal filter cases do not appear on Home merely
    /// because the enum remains `CaseIterable`.
    public static let homeMenuOptions: [Self] = [.all, .nonTask]

    public var tasksQueryValue: String {
        switch self {
        case .all: return "include"
        case .nonTask: return "exclude"
        }
    }

    public var displayName: String {
        switch self {
        case .all: return "All"
        case .nonTask: return "Chats"
        }
    }

    /// A shared non-default status label for Home chrome and section copy.
    public var activeStatusLabel: String? {
        switch self {
        case .all: return nil
        case .nonTask: return "Chats"
        }
    }
}

public struct GaryxRecentThreadFeedPresentation: Equatable, Sendable {
    public var isPrimed: Bool
    public var isRefreshingHead: Bool
    public var headFailure: Bool
    public var footerState: GaryxHomeLoadMoreFooterState

    public init(
        isPrimed: Bool = false,
        isRefreshingHead: Bool = false,
        headFailure: Bool = false,
        footerState: GaryxHomeLoadMoreFooterState = .hidden
    ) {
        self.isPrimed = isPrimed
        self.isRefreshingHead = isRefreshingHead
        self.headFailure = headFailure
        self.footerState = footerState
    }

    public var showsInitialSkeleton: Bool {
        !isPrimed && isRefreshingHead && !headFailure
    }
}

public struct GaryxRecentThreadRefreshTicket: Equatable, Sendable {
    public let filter: GaryxRecentThreadFilter
    public let pagerTicket: GaryxThreadListRefreshTicket
}

public struct GaryxRecentThreadLoadMoreTicket: Equatable, Sendable {
    public let filter: GaryxRecentThreadFilter
    public let pagerTicket: GaryxThreadListLoadMoreTicket
    public let cursor: String

    public var limit: Int { pagerTicket.limit }
}

public struct GaryxRecentThreadFeedState: Equatable, Sendable {
    public private(set) var orderedThreadIds: [String]
    public private(set) var isPrimed: Bool
    public private(set) var headFailure: Bool
    public private(set) var pager: GaryxHomeThreadListPager
    public private(set) var nextCursor: String?

    public init(pageLimit: Int, overlap: Int) {
        orderedThreadIds = []
        isPrimed = false
        headFailure = false
        pager = GaryxHomeThreadListPager(pageLimit: pageLimit, overlap: overlap)
        nextCursor = nil
    }

    public var presentation: GaryxRecentThreadFeedPresentation {
        GaryxRecentThreadFeedPresentation(
            isPrimed: isPrimed,
            isRefreshingHead: pager.isRefreshingHead,
            headFailure: headFailure,
            footerState: pager.footerState
        )
    }

    fileprivate mutating func requestRefresh() -> GaryxThreadListRefreshTicket? {
        guard let ticket = pager.requestRefresh() else { return nil }
        headFailure = false
        return ticket
    }

    fileprivate mutating func completeRefresh(
        _ ticket: GaryxThreadListRefreshTicket,
        pageIds: [String],
        pageCount: Int,
        hasMore: Bool,
        nextCursor responseCursor: String?
    ) -> GaryxThreadListRefreshCompletion {
        let previousCursor = nextCursor
        let completion = pager.completeRefresh(
            ticket,
            pageOffset: 0,
            pageCount: pageCount,
            hasMore: hasMore
        )
        guard case .apply(let application) = completion else { return completion }
        switch application {
        case .replaceHead:
            orderedThreadIds = Self.normalizedIds(pageIds)
            nextCursor = responseCursor
        case .mergeBeyondHead:
            orderedThreadIds = GaryxThreadListPageMerge.mergeHead(
                pageIds: Self.normalizedIds(pageIds),
                existingIds: orderedThreadIds
            )
            nextCursor = previousCursor
        }
        isPrimed = true
        headFailure = false
        return completion
    }

    fileprivate mutating func failRefresh(_ ticket: GaryxThreadListRefreshTicket) {
        let accepted = ticket.epoch == pager.epoch
        pager.failRefresh(ticket)
        if accepted {
            headFailure = true
        }
    }

    fileprivate mutating func requestLoadMore(
        trigger: GaryxThreadListLoadMoreTrigger
    ) -> GaryxThreadListLoadMoreTicket? {
        guard nextCursor != nil else { return nil }
        return pager.requestLoadMore(trigger: trigger)
    }

    fileprivate mutating func retryLoadMore() -> GaryxThreadListLoadMoreTicket? {
        guard nextCursor != nil else { return nil }
        return pager.retryLoadMore()
    }

    fileprivate mutating func completeLoadMore(
        _ ticket: GaryxThreadListLoadMoreTicket,
        pageIds: [String],
        pageCount: Int,
        hasMore: Bool,
        nextCursor responseCursor: String?
    ) -> GaryxThreadListLoadMoreCompletion {
        let pageOffset = pager.nextOffset
        let completion = pager.completeLoadMore(
            ticket,
            pageOffset: pageOffset,
            pageCount: pageCount,
            hasMore: hasMore
        )
        guard completion == .apply else { return completion }
        nextCursor = responseCursor
        orderedThreadIds = GaryxThreadListPageMerge.appendPage(
            pageIds: Self.normalizedIds(pageIds),
            existingIds: orderedThreadIds
        )
        return completion
    }

    fileprivate mutating func failLoadMore(_ ticket: GaryxThreadListLoadMoreTicket) {
        pager.failLoadMore(ticket)
    }

    fileprivate mutating func noteLocalMutation() {
        pager.noteLocalMutation()
    }

    fileprivate mutating func remove(_ threadId: String) {
        orderedThreadIds.removeAll { $0 == threadId }
        noteLocalMutation()
    }

    fileprivate mutating func upsertAtHead(_ threadId: String) {
        orderedThreadIds.removeAll { $0 == threadId }
        orderedThreadIds.insert(threadId, at: 0)
        noteLocalMutation()
    }

    fileprivate mutating func reset() {
        pager.reset()
        orderedThreadIds = []
        isPrimed = false
        headFailure = false
        nextCursor = nil
    }

    private static func normalizedIds(_ ids: [String]) -> [String] {
        var seen = Set<String>()
        return ids.compactMap { rawId in
            let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, seen.insert(id).inserted else { return nil }
            return id
        }
    }
}

public struct GaryxRecentThreadFeeds: Equatable, Sendable {
    public private(set) var selectedFilter: GaryxRecentThreadFilter
    public private(set) var allFeed: GaryxRecentThreadFeedState
    public private(set) var nonTaskFeed: GaryxRecentThreadFeedState

    public init(
        pageLimit: Int,
        overlap: Int,
        selectedFilter: GaryxRecentThreadFilter = .all
    ) {
        self.selectedFilter = selectedFilter
        allFeed = GaryxRecentThreadFeedState(pageLimit: pageLimit, overlap: overlap)
        nonTaskFeed = GaryxRecentThreadFeedState(pageLimit: pageLimit, overlap: overlap)
    }

    public var allRecentThreadIds: [String] { allFeed.orderedThreadIds }

    public var visibleRecentThreadIds: [String] {
        feed(for: selectedFilter).orderedThreadIds
    }

    public var selectedPresentation: GaryxRecentThreadFeedPresentation {
        feed(for: selectedFilter).presentation
    }

    public var selectedPager: GaryxHomeThreadListPager {
        feed(for: selectedFilter).pager
    }

    public func feed(for filter: GaryxRecentThreadFilter) -> GaryxRecentThreadFeedState {
        switch filter {
        case .all: return allFeed
        case .nonTask: return nonTaskFeed
        }
    }

    public mutating func select(_ filter: GaryxRecentThreadFilter) {
        selectedFilter = filter
    }

    public mutating func requestRefresh(
        filter: GaryxRecentThreadFilter? = nil
    ) -> GaryxRecentThreadRefreshTicket? {
        let filter = filter ?? selectedFilter
        switch filter {
        case .all:
            guard let ticket = allFeed.requestRefresh() else { return nil }
            return GaryxRecentThreadRefreshTicket(filter: filter, pagerTicket: ticket)
        case .nonTask:
            guard let ticket = nonTaskFeed.requestRefresh() else { return nil }
            return GaryxRecentThreadRefreshTicket(filter: filter, pagerTicket: ticket)
        }
    }

    @discardableResult
    public mutating func completeRefresh(
        _ ticket: GaryxRecentThreadRefreshTicket,
        pageIds: [String],
        pageCount: Int,
        hasMore: Bool,
        nextCursor: String?
    ) -> GaryxThreadListRefreshCompletion {
        switch ticket.filter {
        case .all:
            return allFeed.completeRefresh(
                ticket.pagerTicket,
                pageIds: pageIds,
                pageCount: pageCount,
                hasMore: hasMore,
                nextCursor: nextCursor
            )
        case .nonTask:
            return nonTaskFeed.completeRefresh(
                ticket.pagerTicket,
                pageIds: pageIds,
                pageCount: pageCount,
                hasMore: hasMore,
                nextCursor: nextCursor
            )
        }
    }

    public mutating func failRefresh(_ ticket: GaryxRecentThreadRefreshTicket) {
        switch ticket.filter {
        case .all: allFeed.failRefresh(ticket.pagerTicket)
        case .nonTask: nonTaskFeed.failRefresh(ticket.pagerTicket)
        }
    }

    public mutating func requestLoadMore(
        trigger: GaryxThreadListLoadMoreTrigger
    ) -> GaryxRecentThreadLoadMoreTicket? {
        switch selectedFilter {
        case .all:
            guard let cursor = allFeed.nextCursor else { return nil }
            guard let ticket = allFeed.requestLoadMore(trigger: trigger) else { return nil }
            return GaryxRecentThreadLoadMoreTicket(filter: .all, pagerTicket: ticket, cursor: cursor)
        case .nonTask:
            guard let cursor = nonTaskFeed.nextCursor else { return nil }
            guard let ticket = nonTaskFeed.requestLoadMore(trigger: trigger) else { return nil }
            return GaryxRecentThreadLoadMoreTicket(filter: .nonTask, pagerTicket: ticket, cursor: cursor)
        }
    }

    public mutating func retryLoadMore() -> GaryxRecentThreadLoadMoreTicket? {
        switch selectedFilter {
        case .all:
            guard let cursor = allFeed.nextCursor else { return nil }
            guard let ticket = allFeed.retryLoadMore() else { return nil }
            return GaryxRecentThreadLoadMoreTicket(filter: .all, pagerTicket: ticket, cursor: cursor)
        case .nonTask:
            guard let cursor = nonTaskFeed.nextCursor else { return nil }
            guard let ticket = nonTaskFeed.retryLoadMore() else { return nil }
            return GaryxRecentThreadLoadMoreTicket(filter: .nonTask, pagerTicket: ticket, cursor: cursor)
        }
    }

    @discardableResult
    public mutating func completeLoadMore(
        _ ticket: GaryxRecentThreadLoadMoreTicket,
        pageIds: [String],
        pageCount: Int,
        hasMore: Bool,
        nextCursor: String?
    ) -> GaryxThreadListLoadMoreCompletion {
        switch ticket.filter {
        case .all:
            return allFeed.completeLoadMore(
                ticket.pagerTicket,
                pageIds: pageIds,
                pageCount: pageCount,
                hasMore: hasMore,
                nextCursor: nextCursor
            )
        case .nonTask:
            return nonTaskFeed.completeLoadMore(
                ticket.pagerTicket,
                pageIds: pageIds,
                pageCount: pageCount,
                hasMore: hasMore,
                nextCursor: nextCursor
            )
        }
    }

    public mutating func failLoadMore(_ ticket: GaryxRecentThreadLoadMoreTicket) {
        switch ticket.filter {
        case .all: allFeed.failLoadMore(ticket.pagerTicket)
        case .nonTask: nonTaskFeed.failLoadMore(ticket.pagerTicket)
        }
    }

    public mutating func noteLocalMutation() {
        allFeed.noteLocalMutation()
        nonTaskFeed.noteLocalMutation()
    }

    public mutating func removeThread(_ rawThreadId: String) {
        let threadId = rawThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !threadId.isEmpty else { return }
        allFeed.remove(threadId)
        nonTaskFeed.remove(threadId)
    }

    public mutating func upsertChat(threadId rawThreadId: String) {
        let threadId = rawThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !threadId.isEmpty else { return }
        allFeed.upsertAtHead(threadId)
        nonTaskFeed.upsertAtHead(threadId)
    }

    /// Clears Gateway-owned page data while retaining the app-global viewing
    /// preference. Resetting each pager still advances its epoch, so results
    /// issued against the previous Gateway cannot commit.
    public mutating func resetFeedData() {
        allFeed.reset()
        nonTaskFeed.reset()
    }
}
