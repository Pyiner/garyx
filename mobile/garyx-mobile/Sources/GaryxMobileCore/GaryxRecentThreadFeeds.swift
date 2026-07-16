import Foundation

public enum GaryxRecentThreadFilter: String, CaseIterable, Equatable, Hashable, Sendable {
    case all
    case nonTask

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

public enum GaryxRecentThreadRefreshMode: Equatable, Sendable {
    case rangeFill
    case replacement
}

public struct GaryxRecentThreadRefreshTicket: Equatable, Sendable {
    public let filter: GaryxRecentThreadFilter
    public let pagerTicket: GaryxThreadListRefreshTicket
    public let gatewayScope: String
    public let runtimeEpoch: UInt64
    public let mode: GaryxRecentThreadRefreshMode
    public let oldHeadActivitySeq: Int64?
    public let forceReplacementGeneration: UInt64
}

public struct GaryxRecentThreadLoadMoreTicket: Equatable, Sendable {
    public let filter: GaryxRecentThreadFilter
    public let pagerTicket: GaryxThreadListLoadMoreTicket
    public let gatewayScope: String
    public let runtimeEpoch: UInt64
    public let cursor: String

    public var limit: Int { pagerTicket.limit }
}

public struct GaryxRecentThreadFeedRow: Equatable, Sendable {
    public var id: String
    public var activitySeq: Int64

    public init(id: String, activitySeq: Int64) {
        self.id = id
        self.activitySeq = activitySeq
    }
}

public struct GaryxRecentThreadFeedPage: Equatable, Sendable {
    public var storeIncarnationId: String
    public var serverBootId: String
    public var rows: [GaryxRecentThreadFeedRow]
    public var hasMore: Bool
    public var nextCursor: String?

    public init(
        storeIncarnationId: String,
        serverBootId: String,
        rows: [GaryxRecentThreadFeedRow],
        hasMore: Bool,
        nextCursor: String?
    ) {
        self.storeIncarnationId = storeIncarnationId
        self.serverBootId = serverBootId
        self.rows = rows
        self.hasMore = hasMore
        self.nextCursor = nextCursor
    }

    public init(_ page: GaryxRecentThreadsPage) {
        self.init(
            storeIncarnationId: page.storeIncarnationId,
            serverBootId: page.serverBootId,
            rows: page.threads.compactMap { thread in
                guard let activitySeq = thread.activitySeq else { return nil }
                return GaryxRecentThreadFeedRow(id: thread.id, activitySeq: activitySeq)
            },
            hasMore: page.hasMore,
            nextCursor: page.nextCursor
        )
    }

    public var headActivitySeq: Int64? { rows.first?.activitySeq }
}

public struct GaryxRecentThreadRefreshBundle: Equatable, Sendable {
    public var primaryPages: [GaryxRecentThreadFeedPage]
    public var verificationPage: GaryxRecentThreadFeedPage
    public var immediatePages: [GaryxRecentThreadFeedPage]?
    public var immediateVerificationPage: GaryxRecentThreadFeedPage?

    public init(
        primaryPages: [GaryxRecentThreadFeedPage],
        verificationPage: GaryxRecentThreadFeedPage,
        immediatePages: [GaryxRecentThreadFeedPage]? = nil,
        immediateVerificationPage: GaryxRecentThreadFeedPage? = nil
    ) {
        self.primaryPages = primaryPages
        self.verificationPage = verificationPage
        self.immediatePages = immediatePages
        self.immediateVerificationPage = immediateVerificationPage
    }
}

public enum GaryxRecentThreadFeedCompletion: Equatable, Sendable {
    case applied
    case abandonedStaleEpoch
    case abandonedLocalMutation
    case forceReplacement
}

public enum GaryxRecentThreadRangeFill {
    public static let maxChainPages = 5
    public static let replacementCycleInterval = 30

    public static func needsNextPage(
        mode: GaryxRecentThreadRefreshMode,
        oldHeadActivitySeq: Int64?,
        pages: [GaryxRecentThreadFeedPage]
    ) -> Bool {
        guard let last = pages.last,
              last.hasMore,
              pages.count < maxChainPages else { return false }
        guard mode == .rangeFill, let oldHeadActivitySeq else { return true }
        guard let tail = last.rows.last?.activitySeq else { return false }
        return tail > oldHeadActivitySeq
    }

    public static func verificationObservedNewerHead(
        chainFirstHead: Int64?,
        verificationPage: GaryxRecentThreadFeedPage
    ) -> Bool {
        guard let verificationHead = verificationPage.headActivitySeq else { return false }
        return chainFirstHead.map { verificationHead > $0 } ?? true
    }
}

public struct GaryxRecentThreadFeedState: Equatable, Sendable {
    public private(set) var orderedThreadIds: [String]
    public private(set) var isPrimed: Bool
    public private(set) var headFailure: Bool
    public private(set) var pager: GaryxHomeThreadListPager
    public private(set) var nextCursor: String?
    public private(set) var storeIncarnationId: String?
    public private(set) var serverBootId: String?
    public private(set) var headActivitySeq: Int64?
    public private(set) var refreshCycle: Int
    public private(set) var forceReplacementPending: Bool
    public private(set) var forceReplacementGeneration: UInt64
    public private(set) var trailingDirty: Bool

    public init(pageLimit: Int, overlap: Int) {
        orderedThreadIds = []
        isPrimed = false
        headFailure = false
        pager = GaryxHomeThreadListPager(pageLimit: pageLimit, overlap: overlap)
        nextCursor = nil
        storeIncarnationId = nil
        serverBootId = nil
        headActivitySeq = nil
        refreshCycle = 0
        forceReplacementPending = false
        forceReplacementGeneration = 0
        trailingDirty = false
    }

    public var presentation: GaryxRecentThreadFeedPresentation {
        GaryxRecentThreadFeedPresentation(
            isPrimed: isPrimed,
            isRefreshingHead: pager.isRefreshingHead,
            headFailure: headFailure,
            footerState: pager.footerState
        )
    }

    fileprivate mutating func requestRefresh(
        gatewayScope: String,
        runtimeEpoch: UInt64,
        forceReplacement: Bool
    ) -> GaryxRecentThreadRefreshTicket? {
        guard !pager.isLoadingMore, let ticket = pager.requestRefresh() else { return nil }
        headFailure = false
        let periodicReplacement = (refreshCycle + 1)
            % GaryxRecentThreadRangeFill.replacementCycleInterval == 0
        let mode: GaryxRecentThreadRefreshMode = forceReplacement
            || forceReplacementPending
            || !isPrimed
            || periodicReplacement
            ? .replacement
            : .rangeFill
        return GaryxRecentThreadRefreshTicket(
            filter: .all, // Feed owner replaces this value.
            pagerTicket: ticket,
            gatewayScope: gatewayScope,
            runtimeEpoch: runtimeEpoch,
            mode: mode,
            oldHeadActivitySeq: headActivitySeq,
            forceReplacementGeneration: forceReplacementGeneration
        )
    }

    fileprivate mutating func completeRefresh(
        _ ticket: GaryxRecentThreadRefreshTicket,
        bundle: GaryxRecentThreadRefreshBundle
    ) -> GaryxRecentThreadFeedCompletion {
        guard !bundle.primaryPages.isEmpty else {
            pager.failRefresh(ticket.pagerTicket)
            headFailure = true
            return .abandonedStaleEpoch
        }
        let allPages = bundle.primaryPages
            + [bundle.verificationPage]
            + (bundle.immediatePages ?? [])
            + [bundle.immediateVerificationPage].compactMap { $0 }
        guard let identity = Self.consistentIdentity(allPages) else {
            pager.failRefresh(ticket.pagerTicket)
            markForceReplacement()
            return .forceReplacement
        }
        if let storeIncarnationId, storeIncarnationId != identity.storeIncarnationId {
            pager.failRefresh(ticket.pagerTicket)
            markForceReplacement()
            return .forceReplacement
        }
        if let serverBootId,
           serverBootId != identity.serverBootId,
           ticket.mode != .replacement {
            pager.failRefresh(ticket.pagerTicket)
            markForceReplacement()
            return .forceReplacement
        }

        var primary = applyChain(
            ticket: ticket,
            pages: bundle.primaryPages,
            existingIds: orderedThreadIds,
            existingCursor: nextCursor
        )
        let primaryHead = bundle.primaryPages.first?.headActivitySeq
        let needsImmediate = GaryxRecentThreadRangeFill.verificationObservedNewerHead(
            chainFirstHead: primaryHead,
            verificationPage: bundle.verificationPage
        )
        var continuedMotion = false
        if needsImmediate, let immediatePages = bundle.immediatePages,
           !immediatePages.isEmpty {
            let immediateTicket = GaryxRecentThreadRefreshTicket(
                filter: ticket.filter,
                pagerTicket: ticket.pagerTicket,
                gatewayScope: ticket.gatewayScope,
                runtimeEpoch: ticket.runtimeEpoch,
                mode: .rangeFill,
                oldHeadActivitySeq: primaryHead,
                forceReplacementGeneration: ticket.forceReplacementGeneration
            )
            let immediate = applyChain(
                ticket: immediateTicket,
                pages: immediatePages,
                existingIds: primary.ids,
                existingCursor: primary.cursor
            )
            primary = (
                ids: immediate.ids,
                cursor: immediate.cursor,
                hasMore: immediate.hasMore,
                replacement: primary.replacement || immediate.replacement
            )
            if let verification = bundle.immediateVerificationPage {
                continuedMotion = GaryxRecentThreadRangeFill.verificationObservedNewerHead(
                    chainFirstHead: immediatePages.first?.headActivitySeq,
                    verificationPage: verification
                )
            } else {
                continuedMotion = true
            }
        } else if needsImmediate {
            continuedMotion = true
        }

        switch pager.completeRangeRefresh(
            ticket.pagerTicket,
            committedCount: primary.ids.count,
            hasMore: primary.hasMore,
            replacementCommitted: primary.replacement
        ) {
        case .abandonedStaleEpoch:
            return .abandonedStaleEpoch
        case .abandonedLocalMutation:
            return .abandonedLocalMutation
        case .apply:
            let replacementRequestedAfterDispatch = forceReplacementPending
                && forceReplacementGeneration != ticket.forceReplacementGeneration
            orderedThreadIds = primary.ids
            nextCursor = primary.cursor
            storeIncarnationId = identity.storeIncarnationId
            serverBootId = identity.serverBootId
            headActivitySeq = (bundle.immediatePages?.first ?? bundle.primaryPages.first)?
                .headActivitySeq
            refreshCycle += 1
            forceReplacementPending = replacementRequestedAfterDispatch
            trailingDirty = continuedMotion
            isPrimed = true
            headFailure = false
            return replacementRequestedAfterDispatch ? .forceReplacement : .applied
        }
    }

    fileprivate mutating func failRefresh(_ ticket: GaryxRecentThreadRefreshTicket) {
        let accepted = ticket.pagerTicket.epoch == pager.epoch
        pager.failRefresh(ticket.pagerTicket)
        if accepted { headFailure = true }
    }

    fileprivate mutating func requestLoadMore(
        trigger: GaryxThreadListLoadMoreTrigger,
        gatewayScope: String,
        runtimeEpoch: UInt64
    ) -> GaryxRecentThreadLoadMoreTicket? {
        guard !pager.isRefreshingHead,
              !forceReplacementPending,
              let cursor = nextCursor,
              let ticket = pager.requestLoadMore(trigger: trigger) else { return nil }
        return GaryxRecentThreadLoadMoreTicket(
            filter: .all,
            pagerTicket: ticket,
            gatewayScope: gatewayScope,
            runtimeEpoch: runtimeEpoch,
            cursor: cursor
        )
    }

    fileprivate mutating func retryLoadMore(
        gatewayScope: String,
        runtimeEpoch: UInt64
    ) -> GaryxRecentThreadLoadMoreTicket? {
        guard !pager.isRefreshingHead,
              !forceReplacementPending,
              let cursor = nextCursor,
              let ticket = pager.retryLoadMore() else { return nil }
        return GaryxRecentThreadLoadMoreTicket(
            filter: .all,
            pagerTicket: ticket,
            gatewayScope: gatewayScope,
            runtimeEpoch: runtimeEpoch,
            cursor: cursor
        )
    }

    fileprivate mutating func completeLoadMore(
        _ ticket: GaryxRecentThreadLoadMoreTicket,
        page: GaryxRecentThreadFeedPage
    ) -> GaryxRecentThreadFeedCompletion {
        if let storeIncarnationId, storeIncarnationId != page.storeIncarnationId {
            pager.failLoadMore(ticket.pagerTicket)
            markForceReplacement()
            return .forceReplacement
        }
        if let serverBootId, serverBootId != page.serverBootId {
            pager.failLoadMore(ticket.pagerTicket)
            markForceReplacement()
            return .forceReplacement
        }
        switch pager.completeLoadMore(
            ticket.pagerTicket,
            pageOffset: pager.nextOffset,
            pageCount: page.rows.count,
            hasMore: page.hasMore
        ) {
        case .abandonedStaleEpoch:
            return .abandonedStaleEpoch
        case .abandonedLocalMutation:
            return .abandonedLocalMutation
        case .apply:
            orderedThreadIds = GaryxThreadListPageMerge.appendPage(
                pageIds: Self.normalizedIds(page.rows.map(\.id)),
                existingIds: orderedThreadIds
            )
            nextCursor = page.nextCursor
            storeIncarnationId = page.storeIncarnationId
            serverBootId = page.serverBootId
            return .applied
        }
    }

    fileprivate mutating func failLoadMore(_ ticket: GaryxRecentThreadLoadMoreTicket) {
        pager.failLoadMore(ticket.pagerTicket)
    }

    fileprivate mutating func noteLocalMutation() { pager.noteLocalMutation() }

    fileprivate mutating func remove(_ threadId: String) {
        orderedThreadIds.removeAll { $0 == threadId }
        noteLocalMutation()
    }

    fileprivate mutating func upsertAtHead(_ threadId: String) {
        orderedThreadIds.removeAll { $0 == threadId }
        orderedThreadIds.insert(threadId, at: 0)
        headActivitySeq = nil
        noteLocalMutation()
    }

    fileprivate mutating func markForceReplacement() {
        forceReplacementGeneration &+= 1
        forceReplacementPending = true
        trailingDirty = false
    }

    fileprivate mutating func reset() {
        pager.reset()
        orderedThreadIds = []
        isPrimed = false
        headFailure = false
        nextCursor = nil
        storeIncarnationId = nil
        serverBootId = nil
        headActivitySeq = nil
        refreshCycle = 0
        forceReplacementPending = false
        forceReplacementGeneration = 0
        trailingDirty = false
    }

    private func applyChain(
        ticket: GaryxRecentThreadRefreshTicket,
        pages: [GaryxRecentThreadFeedPage],
        existingIds: [String],
        existingCursor: String?
    ) -> (ids: [String], cursor: String?, hasMore: Bool, replacement: Bool) {
        let pageIds = Self.normalizedIds(pages.flatMap { $0.rows.map(\.id) })
        let last = pages.last
        let reachedAnchor = ticket.oldHeadActivitySeq.map { anchor in
            last?.rows.last.map { $0.activitySeq <= anchor } ?? false
        } ?? false
        let exhaustedBeforeAnchor = last?.hasMore == false && !reachedAnchor
        let exceededWindow = pages.count >= GaryxRecentThreadRangeFill.maxChainPages
            && !reachedAnchor
        let replacement = ticket.mode == .replacement
            || ticket.oldHeadActivitySeq == nil
            || exhaustedBeforeAnchor
            || exceededWindow
        if replacement {
            return (pageIds, last?.nextCursor, last?.hasMore ?? false, true)
        }
        return (
            GaryxThreadListPageMerge.mergeHead(
                pageIds: pageIds,
                existingIds: existingIds
            ),
            existingCursor,
            existingCursor != nil,
            false
        )
    }

    private static func consistentIdentity(
        _ pages: [GaryxRecentThreadFeedPage]
    ) -> (storeIncarnationId: String, serverBootId: String)? {
        guard let first = pages.first,
              pages.allSatisfy({
                  $0.storeIncarnationId == first.storeIncarnationId
                      && $0.serverBootId == first.serverBootId
              }) else { return nil }
        return (first.storeIncarnationId, first.serverBootId)
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
    public var visibleRecentThreadIds: [String] { feed(for: selectedFilter).orderedThreadIds }
    public var selectedPresentation: GaryxRecentThreadFeedPresentation {
        feed(for: selectedFilter).presentation
    }
    public var selectedPager: GaryxHomeThreadListPager { feed(for: selectedFilter).pager }

    public func feed(for filter: GaryxRecentThreadFilter) -> GaryxRecentThreadFeedState {
        switch filter {
        case .all: return allFeed
        case .nonTask: return nonTaskFeed
        }
    }

    public mutating func select(_ filter: GaryxRecentThreadFilter) { selectedFilter = filter }

    public mutating func requestRefresh(
        filter: GaryxRecentThreadFilter? = nil,
        gatewayScope: String = "",
        runtimeEpoch: UInt64 = 0,
        forceReplacement: Bool = false
    ) -> GaryxRecentThreadRefreshTicket? {
        let filter = filter ?? selectedFilter
        switch filter {
        case .all:
            guard let ticket = allFeed.requestRefresh(
                gatewayScope: gatewayScope,
                runtimeEpoch: runtimeEpoch,
                forceReplacement: forceReplacement
            ) else { return nil }
            return GaryxRecentThreadRefreshTicket(
                filter: filter,
                pagerTicket: ticket.pagerTicket,
                gatewayScope: ticket.gatewayScope,
                runtimeEpoch: ticket.runtimeEpoch,
                mode: ticket.mode,
                oldHeadActivitySeq: ticket.oldHeadActivitySeq,
                forceReplacementGeneration: ticket.forceReplacementGeneration
            )
        case .nonTask:
            guard let ticket = nonTaskFeed.requestRefresh(
                gatewayScope: gatewayScope,
                runtimeEpoch: runtimeEpoch,
                forceReplacement: forceReplacement
            ) else { return nil }
            return GaryxRecentThreadRefreshTicket(
                filter: filter,
                pagerTicket: ticket.pagerTicket,
                gatewayScope: ticket.gatewayScope,
                runtimeEpoch: ticket.runtimeEpoch,
                mode: ticket.mode,
                oldHeadActivitySeq: ticket.oldHeadActivitySeq,
                forceReplacementGeneration: ticket.forceReplacementGeneration
            )
        }
    }

    @discardableResult
    public mutating func completeRefresh(
        _ ticket: GaryxRecentThreadRefreshTicket,
        bundle: GaryxRecentThreadRefreshBundle
    ) -> GaryxRecentThreadFeedCompletion {
        switch ticket.filter {
        case .all: return allFeed.completeRefresh(ticket, bundle: bundle)
        case .nonTask: return nonTaskFeed.completeRefresh(ticket, bundle: bundle)
        }
    }

    public mutating func failRefresh(_ ticket: GaryxRecentThreadRefreshTicket) {
        switch ticket.filter {
        case .all: allFeed.failRefresh(ticket)
        case .nonTask: nonTaskFeed.failRefresh(ticket)
        }
    }

    public mutating func requestLoadMore(
        trigger: GaryxThreadListLoadMoreTrigger,
        gatewayScope: String = "",
        runtimeEpoch: UInt64 = 0
    ) -> GaryxRecentThreadLoadMoreTicket? {
        switch selectedFilter {
        case .all:
            guard let ticket = allFeed.requestLoadMore(
                trigger: trigger,
                gatewayScope: gatewayScope,
                runtimeEpoch: runtimeEpoch
            ) else { return nil }
            return GaryxRecentThreadLoadMoreTicket(
                filter: .all,
                pagerTicket: ticket.pagerTicket,
                gatewayScope: ticket.gatewayScope,
                runtimeEpoch: ticket.runtimeEpoch,
                cursor: ticket.cursor
            )
        case .nonTask:
            guard let ticket = nonTaskFeed.requestLoadMore(
                trigger: trigger,
                gatewayScope: gatewayScope,
                runtimeEpoch: runtimeEpoch
            ) else { return nil }
            return GaryxRecentThreadLoadMoreTicket(
                filter: .nonTask,
                pagerTicket: ticket.pagerTicket,
                gatewayScope: ticket.gatewayScope,
                runtimeEpoch: ticket.runtimeEpoch,
                cursor: ticket.cursor
            )
        }
    }

    public mutating func retryLoadMore(
        gatewayScope: String = "",
        runtimeEpoch: UInt64 = 0
    ) -> GaryxRecentThreadLoadMoreTicket? {
        switch selectedFilter {
        case .all:
            guard let ticket = allFeed.retryLoadMore(
                gatewayScope: gatewayScope,
                runtimeEpoch: runtimeEpoch
            ) else { return nil }
            return GaryxRecentThreadLoadMoreTicket(
                filter: .all,
                pagerTicket: ticket.pagerTicket,
                gatewayScope: ticket.gatewayScope,
                runtimeEpoch: ticket.runtimeEpoch,
                cursor: ticket.cursor
            )
        case .nonTask:
            guard let ticket = nonTaskFeed.retryLoadMore(
                gatewayScope: gatewayScope,
                runtimeEpoch: runtimeEpoch
            ) else { return nil }
            return GaryxRecentThreadLoadMoreTicket(
                filter: .nonTask,
                pagerTicket: ticket.pagerTicket,
                gatewayScope: ticket.gatewayScope,
                runtimeEpoch: ticket.runtimeEpoch,
                cursor: ticket.cursor
            )
        }
    }

    @discardableResult
    public mutating func completeLoadMore(
        _ ticket: GaryxRecentThreadLoadMoreTicket,
        page: GaryxRecentThreadFeedPage
    ) -> GaryxRecentThreadFeedCompletion {
        switch ticket.filter {
        case .all: return allFeed.completeLoadMore(ticket, page: page)
        case .nonTask: return nonTaskFeed.completeLoadMore(ticket, page: page)
        }
    }

    public mutating func failLoadMore(_ ticket: GaryxRecentThreadLoadMoreTicket) {
        switch ticket.filter {
        case .all: allFeed.failLoadMore(ticket)
        case .nonTask: nonTaskFeed.failLoadMore(ticket)
        }
    }

    public mutating func forceReplacement() {
        allFeed.markForceReplacement()
        nonTaskFeed.markForceReplacement()
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

    public mutating func resetFeedData() {
        allFeed.reset()
        nonTaskFeed.reset()
    }
}
