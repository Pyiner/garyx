import XCTest
@testable import GaryxMobileCore

@MainActor
final class GaryxThreadSummaryCacheTests: XCTestCase {
    func testPinnedMembershipCanExceedLRUCapacityAndScrollBackReadsAll501() {
        let cache = GaryxThreadSummaryCache()
        XCTAssertEqual(cache.unpinnedCapacity, 500)
        let owner = GaryxThreadSummaryLeaseOwner(cache: cache)
        let rows = (0...500).map(thread)

        owner.replacePage(
            ownerId: "workspace",
            threadIds: rows.map(\.id),
            summaries: rows
        )

        XCTAssertEqual(cache.count, 501)
        XCTAssertEqual(cache.pinnedCount, 501)
        for row in rows.reversed() {
            XCTAssertEqual(cache.summary(for: row.id), row)
        }

        owner.removePage(ownerId: "workspace")
        XCTAssertEqual(cache.pinnedCount, 0)
        XCTAssertEqual(cache.unpinnedCount, 500)
        XCTAssertEqual(cache.count, 500)
    }

    func testOverlappingSourcesReleaseOnlyTheirOwnReference() {
        let cache = GaryxThreadSummaryCache(unpinnedCapacity: 0)
        let owner = GaryxThreadSummaryLeaseOwner(cache: cache)
        let row = thread(1)

        owner.replacePage(ownerId: "page", threadIds: [row.id], summaries: [row])
        owner.swapSelectedThread(row)
        XCTAssertEqual(cache.pinCount(for: row.id), 2)

        owner.removePage(ownerId: "page")
        XCTAssertEqual(cache.pinCount(for: row.id), 1)
        XCTAssertEqual(cache.summary(for: row.id), row)

        owner.swapSelectedThread(nil)
        XCTAssertEqual(cache.pinCount(for: row.id), 0)
        XCTAssertNil(cache.summary(for: row.id))
    }

    func testReferenceOwnerAliasAndRepeatedSwapCannotReleaseNewLeaseViaOldValue() {
        let cache = GaryxThreadSummaryCache(unpinnedCapacity: 0)
        let owner = GaryxThreadSummaryLeaseOwner(cache: cache)
        let alias = owner
        let first = thread(1)
        let replacement = thread(2)

        owner.replaceFeed(ownerId: "feed", threadIds: [first.id], summaries: [first])
        alias.replaceFeed(
            ownerId: "feed",
            threadIds: [replacement.id],
            summaries: [replacement]
        )

        XCTAssertNil(cache.summary(for: first.id))
        XCTAssertEqual(cache.summary(for: replacement.id), replacement)
        XCTAssertEqual(cache.pinCount(for: replacement.id), 1)
        XCTAssertEqual(alias.activeLeaseCount, 1)
    }

    func testDidSetOldValueRetentionCannotCopyOrPrematurelyReleaseLeaseState() {
        let cache = GaryxThreadSummaryCache(unpinnedCapacity: 0)
        let owner = GaryxThreadSummaryLeaseOwner(cache: cache)
        let holder = LeaseOwnerHolder(owner: owner)
        let row = thread(7)
        holder.owner.replacePage(ownerId: "page", threadIds: [row.id], summaries: [row])

        let alias = holder.owner
        holder.owner = alias
        XCTAssertTrue(holder.observedOldOwners.first === owner)
        XCTAssertTrue(holder.owner === owner)
        XCTAssertEqual(cache.pinCount(for: row.id), 1)
        XCTAssertEqual(cache.summary(for: row.id), row)
    }

    func testPickerQuerySwapAcquiresNewPinsBeforeReleasingOldAndSelectedIsIndependent() {
        let cache = GaryxThreadSummaryCache(unpinnedCapacity: 0)
        let owner = GaryxThreadSummaryLeaseOwner(cache: cache)
        let shared = thread(1)
        let next = thread(2)
        let oldOnly = thread(3)

        owner.replacePickerQuery(
            instanceId: 1,
            threadIds: [shared.id, oldOnly.id],
            summaries: [shared, oldOnly]
        )
        owner.swapPickerSelectedTarget(shared)
        owner.replacePickerQuery(instanceId: 2, threadIds: [shared.id, next.id], summaries: [shared, next])

        XCTAssertEqual(cache.pinCount(for: shared.id), 2)
        XCTAssertEqual(cache.pinCount(for: next.id), 1)
        XCTAssertNil(cache.summary(for: oldOnly.id))
        owner.closePicker()
        XCTAssertEqual(cache.count, 0)
        XCTAssertEqual(owner.activeLeaseCount, 0)
    }

    func testEveryReleasePointIncludingEarlyReturnFamiliesDropsItsLease() {
        let cache = GaryxThreadSummaryCache(unpinnedCapacity: 0)
        let owner = GaryxThreadSummaryLeaseOwner(cache: cache)

        func assertReleased(
            _ id: Int,
            acquire: (GaryxThreadSummary) -> Void,
            release: () -> Void,
            file: StaticString = #filePath,
            line: UInt = #line
        ) {
            let row = thread(id)
            acquire(row)
            XCTAssertEqual(cache.pinCount(for: row.id), 1, file: file, line: line)
            release()
            XCTAssertEqual(cache.pinCount(for: row.id), 0, file: file, line: line)
            XCTAssertNil(cache.summary(for: row.id), file: file, line: line)
        }

        assertReleased(1) {
            owner.replacePage(ownerId: "page", threadIds: [$0.id], summaries: [$0])
        } release: {
            owner.removePage(ownerId: "page")
        }
        assertReleased(2) {
            owner.replaceFeed(ownerId: "evict", threadIds: [$0.id], summaries: [$0])
        } release: {
            owner.evictFeed(ownerId: "evict")
        }
        assertReleased(3) {
            owner.replaceFeed(ownerId: "reset", threadIds: [$0.id], summaries: [$0])
        } release: {
            owner.resetFeeds()
        }
        assertReleased(4) { owner.swapSelectedThread($0) } release: {
            owner.swapSelectedThread(nil) // selected -> draft
        }
        assertReleased(5) {
            owner.beginWidgetWrite(token: "finish", threadIds: [$0.id], summaries: [$0])
        } release: {
            owner.finishWidgetWrite(token: "finish")
        }
        assertReleased(6) {
            owner.beginWidgetWrite(token: "cancel", threadIds: [$0.id], summaries: [$0])
        } release: {
            owner.cancelWidgetWrite(token: "cancel")
        }
        assertReleased(7) {
            owner.beginWidgetWrite(token: "skip", threadIds: [$0.id], summaries: [$0])
        } release: {
            owner.skipWidgetWrite(token: "skip")
        }
        assertReleased(8) {
            owner.replaceComposerReferences(ownerId: "settle", threadIds: [$0.id], summaries: [$0])
        } release: {
            owner.settleComposer(ownerId: "settle")
        }
        assertReleased(9) {
            owner.replaceComposerReferences(ownerId: "cancel", threadIds: [$0.id], summaries: [$0])
        } release: {
            owner.cancelComposer(ownerId: "cancel")
        }
        assertReleased(10) {
            owner.replaceComposerReferences(ownerId: "remove", threadIds: [$0.id], summaries: [$0])
        } release: {
            owner.removeComposer(ownerId: "remove")
        }
        assertReleased(11) {
            owner.replaceBotEntries(groupId: "bot", threadIds: [$0.id], summaries: [$0])
        } release: {
            owner.removeBotEntries(groupId: "bot")
        }

        let gateway = thread(12)
        owner.replaceFeed(ownerId: "gateway", threadIds: [gateway.id], summaries: [gateway])
        owner.swapSelectedThread(gateway)
        owner.resetGatewayScope()
        XCTAssertEqual(cache.count, 0)
        XCTAssertEqual(owner.activeLeaseCount, 0)
    }

    func testPageSelectedAndBotReplacementReleasePriorSlotBeforeCapacityPrune() {
        let cache = GaryxThreadSummaryCache(unpinnedCapacity: 0)
        let owner = GaryxThreadSummaryLeaseOwner(cache: cache)
        let first = thread(20)
        let second = thread(21)

        owner.replacePage(ownerId: "page", threadIds: [first.id], summaries: [first])
        owner.replacePage(ownerId: "page", threadIds: [second.id], summaries: [second])
        XCTAssertNil(cache.summary(for: first.id))
        XCTAssertEqual(cache.pinCount(for: second.id), 1)

        owner.swapSelectedThread(first)
        owner.swapSelectedThread(second)
        XCTAssertNil(cache.summary(for: first.id))
        XCTAssertEqual(cache.pinCount(for: second.id), 2)

        owner.replaceBotEntries(groupId: "bot", threadIds: [first.id], summaries: [first])
        owner.replaceBotEntries(groupId: "bot", threadIds: [second.id], summaries: [second])
        XCTAssertNil(cache.summary(for: first.id))
        XCTAssertEqual(cache.pinCount(for: second.id), 3)
    }

    func testCopyablePagerAndFeedStatesRemainSendableAndCarryNoLeaseLifetime() {
        func assertSendable<Value: Sendable>(_: Value.Type) {}
        assertSendable(GaryxHomeThreadListPager.self)
        assertSendable(GaryxRecentThreadFeeds.self)
        assertSendable(GaryxThreadSummaryMembershipProvider.self)

        var pager = GaryxHomeThreadListPager(pageLimit: 30, overlap: 5)
        let copy = pager
        pager.noteLocalMutation()
        XCTAssertNotEqual(pager, copy)
    }

    private func thread(_ index: Int) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: "thread::\(index)",
            title: "Thread \(index)",
            createdAt: nil,
            updatedAt: nil,
            lastMessagePreview: "",
            workspacePath: nil,
            messageCount: nil,
            agentId: nil,
            providerType: nil,
            recentRunId: nil,
            activeRunId: nil,
            runState: nil,
            worktreePath: nil
        )
    }

    @MainActor
    private final class LeaseOwnerHolder {
        var observedOldOwners: [GaryxThreadSummaryLeaseOwner] = []
        var owner: GaryxThreadSummaryLeaseOwner {
            didSet { observedOldOwners.append(oldValue) }
        }

        init(owner: GaryxThreadSummaryLeaseOwner) {
            self.owner = owner
        }
    }
}
