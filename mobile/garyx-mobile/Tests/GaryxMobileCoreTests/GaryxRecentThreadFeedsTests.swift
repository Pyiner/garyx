import XCTest
@testable import GaryxMobileCore

final class GaryxRecentThreadFeedsTests: XCTestCase {
    func testFilterWireMapping() {
        XCTAssertEqual(GaryxRecentThreadFilter.all.tasksQueryValue, "include")
        XCTAssertEqual(GaryxRecentThreadFilter.nonTask.tasksQueryValue, "exclude")
        XCTAssertEqual(GaryxRecentThreadFilter.all.displayName, "All")
        XCTAssertEqual(GaryxRecentThreadFilter.nonTask.displayName, "Chats")
    }

    func testDefaultsToAllAndKeepsAllStableWhenVisibleFilterChanges() {
        var feeds = makeFeeds()
        adoptHead(&feeds, filter: .all, ids: ["task", "chat"])
        adoptHead(&feeds, filter: .nonTask, ids: ["chat"])

        XCTAssertEqual(feeds.selectedFilter, .all)
        XCTAssertEqual(feeds.visibleRecentThreadIds, ["task", "chat"])
        feeds.select(.nonTask)
        XCTAssertEqual(feeds.visibleRecentThreadIds, ["chat"])
        XCTAssertEqual(feeds.allRecentThreadIds, ["task", "chat"])
    }

    func testLateCompletionWritesTicketFilterNotCurrentSelection() throws {
        var feeds = makeFeeds()
        let allTicket = try XCTUnwrap(feeds.requestRefresh(filter: .all))
        feeds.select(.nonTask)
        let chatsTicket = try XCTUnwrap(feeds.requestRefresh())
        feeds.select(.all)

        XCTAssertEqual(
            feeds.completeRefresh(
                chatsTicket,
                pageIds: ["chat-a", "chat-b"],
                pageOffset: 0,
                pageCount: 2,
                hasMore: false
            ),
            .apply(.replaceHead)
        )
        XCTAssertEqual(feeds.nonTaskFeed.orderedThreadIds, ["chat-a", "chat-b"])
        XCTAssertEqual(feeds.allFeed.orderedThreadIds, [])

        _ = feeds.completeRefresh(
            allTicket,
            pageIds: ["task", "chat-a"],
            pageOffset: 0,
            pageCount: 2,
            hasMore: false
        )
        XCTAssertEqual(feeds.allFeed.orderedThreadIds, ["task", "chat-a"])
    }

    func testResetAbandonsOldEpochAndResetsSelection() throws {
        var feeds = makeFeeds()
        feeds.select(.nonTask)
        let ticket = try XCTUnwrap(feeds.requestRefresh())
        feeds.reset()

        XCTAssertEqual(
            feeds.completeRefresh(
                ticket,
                pageIds: ["old"],
                pageOffset: 0,
                pageCount: 1,
                hasMore: false
            ),
            .abandonedStaleEpoch
        )
        XCTAssertEqual(feeds.selectedFilter, .all)
        XCTAssertTrue(feeds.allRecentThreadIds.isEmpty)
        XCTAssertTrue(feeds.visibleRecentThreadIds.isEmpty)
    }

    func testSuccessfulEmptyPageIsPrimedAndFirstFailureIsUnavailable() throws {
        var feeds = makeFeeds()
        let emptyTicket = try XCTUnwrap(feeds.requestRefresh(filter: .all))
        _ = feeds.completeRefresh(
            emptyTicket,
            pageIds: [],
            pageOffset: 0,
            pageCount: 0,
            hasMore: false
        )
        XCTAssertTrue(feeds.allFeed.isPrimed)
        XCTAssertFalse(feeds.allFeed.headFailure)

        let failedTicket = try XCTUnwrap(feeds.requestRefresh(filter: .nonTask))
        feeds.failRefresh(failedTicket)
        XCTAssertFalse(feeds.nonTaskFeed.isPrimed)
        XCTAssertTrue(feeds.nonTaskFeed.headFailure)
        XCTAssertFalse(feeds.allFeed.headFailure)
    }

    func testAuxiliaryAllRefreshCoalescesAndDoesNotChangeSelectedPhase() throws {
        var feeds = makeFeeds()
        feeds.select(.nonTask)
        let selectedTicket = try XCTUnwrap(feeds.requestRefresh())
        XCTAssertTrue(feeds.selectedPresentation.isRefreshingHead)

        let auxiliaryTicket = try XCTUnwrap(feeds.requestRefresh(filter: .all))
        XCTAssertNil(feeds.requestRefresh(filter: .all))
        XCTAssertEqual(auxiliaryTicket.filter, .all)
        XCTAssertEqual(selectedTicket.filter, .nonTask)

        feeds.failRefresh(auxiliaryTicket)
        XCTAssertTrue(feeds.allFeed.headFailure)
        XCTAssertFalse(feeds.selectedPresentation.headFailure)
        XCTAssertTrue(feeds.selectedPresentation.isRefreshingHead)
    }

    func testAuxiliaryAllSuccessWritesOnlyAllAndDoesNotFinishSelectedRefresh() throws {
        var feeds = makeFeeds()
        feeds.select(.nonTask)
        _ = try XCTUnwrap(feeds.requestRefresh())
        let auxiliaryTicket = try XCTUnwrap(feeds.requestRefresh(filter: .all))

        XCTAssertEqual(
            feeds.completeRefresh(
                auxiliaryTicket,
                pageIds: ["task", "chat"],
                pageOffset: 0,
                pageCount: 2,
                hasMore: false
            ),
            .apply(.replaceHead)
        )
        XCTAssertEqual(feeds.allRecentThreadIds, ["task", "chat"])
        XCTAssertTrue(feeds.nonTaskFeed.orderedThreadIds.isEmpty)
        XCTAssertTrue(feeds.selectedPresentation.isRefreshingHead)
    }

    func testHeadMergeAndOverlappedLoadMoreAreFilterLocal() throws {
        var feeds = makeFeeds()
        adoptHead(&feeds, filter: .all, ids: ["a", "b", "c"], hasMore: true)
        let firstLoad = try XCTUnwrap(feeds.requestLoadMore(trigger: .footer))
        XCTAssertEqual(firstLoad.filter, .all)
        XCTAssertEqual(firstLoad.offset, 0)
        _ = feeds.completeLoadMore(
            firstLoad,
            pageIds: ["a", "b", "c", "d", "e", "f"],
            pageOffset: 0,
            pageCount: 6,
            hasMore: true
        )

        let head = try XCTUnwrap(feeds.requestRefresh(filter: .all))
        _ = feeds.completeRefresh(
            head,
            pageIds: ["new", "a", "b"],
            pageOffset: 0,
            pageCount: 3,
            hasMore: true
        )
        XCTAssertEqual(feeds.allFeed.orderedThreadIds, ["new", "a", "b", "c", "d", "e", "f"])

        let secondLoad = try XCTUnwrap(feeds.requestLoadMore(trigger: .nearTail))
        XCTAssertEqual(secondLoad.offset, 1)
        _ = feeds.completeLoadMore(
            secondLoad,
            pageIds: ["a", "b", "c", "d", "e", "f", "g"],
            pageOffset: 1,
            pageCount: 7,
            hasMore: false
        )
        XCTAssertEqual(feeds.allFeed.orderedThreadIds, ["new", "a", "b", "c", "d", "e", "f", "g"])
        XCTAssertEqual(feeds.allFeed.pager.footerState, .hidden)
        XCTAssertTrue(feeds.nonTaskFeed.orderedThreadIds.isEmpty)
    }

    func testInactiveLoadMoreFailureDoesNotChangeSelectedFooter() throws {
        var feeds = makeFeeds()
        adoptHead(&feeds, filter: .all, ids: ["task", "chat"], hasMore: true)
        adoptHead(&feeds, filter: .nonTask, ids: ["chat"], hasMore: true)
        feeds.select(.all)

        feeds.select(.nonTask)
        let chatsLoad = try XCTUnwrap(feeds.requestLoadMore(trigger: .footer))
        feeds.select(.all)
        feeds.failLoadMore(chatsLoad)

        XCTAssertEqual(feeds.nonTaskFeed.presentation.footerState, .failed)
        XCTAssertEqual(feeds.selectedPresentation.footerState, .idle)
        XCTAssertFalse(feeds.selectedPresentation.headFailure)
    }

    func testLocalRemovalBlocksStalePageAndRollbackRestoresBothOrders() throws {
        var feeds = makeFeeds()
        adoptHead(&feeds, filter: .all, ids: ["task", "chat", "tail"])
        adoptHead(&feeds, filter: .nonTask, ids: ["chat", "tail"])
        let stale = try XCTUnwrap(feeds.requestRefresh(filter: .all))

        let rollback = feeds.removeThread("chat")
        XCTAssertEqual(feeds.allRecentThreadIds, ["task", "tail"])
        XCTAssertEqual(feeds.nonTaskFeed.orderedThreadIds, ["tail"])
        XCTAssertEqual(
            feeds.completeRefresh(
                stale,
                pageIds: ["task", "chat", "tail"],
                pageOffset: 0,
                pageCount: 3,
                hasMore: false
            ),
            .abandonedLocalMutation
        )

        feeds.rollbackRemoval(rollback)
        XCTAssertEqual(feeds.allRecentThreadIds, ["task", "chat", "tail"])
        XCTAssertEqual(feeds.nonTaskFeed.orderedThreadIds, ["chat", "tail"])
    }

    func testChatUpsertTouchesBothFeedsWhileTaskMembershipComesFromServerPages() {
        var feeds = makeFeeds()
        adoptHead(&feeds, filter: .all, ids: ["task", "chat"])
        adoptHead(&feeds, filter: .nonTask, ids: ["chat"])
        feeds.upsertChat(threadId: "new-chat")
        XCTAssertEqual(feeds.allRecentThreadIds, ["new-chat", "task", "chat"])
        XCTAssertEqual(feeds.nonTaskFeed.orderedThreadIds, ["new-chat", "chat"])

        adoptHead(&feeds, filter: .all, ids: ["new-task", "new-chat", "task"])
        adoptHead(&feeds, filter: .nonTask, ids: ["new-chat", "chat"])
        XCTAssertEqual(feeds.allRecentThreadIds.first, "new-task")
        XCTAssertFalse(feeds.nonTaskFeed.orderedThreadIds.contains("new-task"))
    }

    func testPinnedTaskRemainsInPinnedSectionWhenChatsOwnsVisibleRecentIds() {
        let fixture = GaryxHomeListFixture.makeInputs(
            threadCount: 3,
            pinnedCount: 1,
            runningCount: 0
        )
        let input = GaryxHomeThreadSectionsInput(
            threads: fixture.threads,
            agents: fixture.agents,
            automations: fixture.automations,
            pinnedThreadIds: ["thread-0"],
            recentThreadIds: ["thread-1", "thread-2"],
            selectedThreadId: nil
        )
        let sections = GaryxHomeThreadSectionsBuilder.build(input)

        XCTAssertEqual(sections.pinned.map(\.id), ["thread-0"])
        XCTAssertEqual(sections.recent.map(\.id), ["thread-1", "thread-2"])
    }

    private func makeFeeds() -> GaryxRecentThreadFeeds {
        GaryxRecentThreadFeeds(pageLimit: 30, overlap: 5)
    }

    private func adoptHead(
        _ feeds: inout GaryxRecentThreadFeeds,
        filter: GaryxRecentThreadFilter,
        ids: [String],
        hasMore: Bool = false
    ) {
        let ticket = feeds.requestRefresh(filter: filter)!
        _ = feeds.completeRefresh(
            ticket,
            pageIds: ids,
            pageOffset: 0,
            pageCount: ids.count,
            hasMore: hasMore
        )
    }
}
