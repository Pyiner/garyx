import XCTest
@testable import GaryxMobileCore

final class GaryxThreadListPresentationTests: XCTestCase {
    func testActionPlannerProducesCompleteContextMenuForMutableThread() {
        let full = GaryxThreadRowCapabilities(
            canOpen: true,
            canPin: true,
            canArchive: true,
            favorite: .addAndRemove,
            archiveStrategy: .thread
        )
        XCTAssertEqual(
            GaryxThreadRowActionPlanner.actions(
                capabilities: full,
                isPinned: false,
                isFavorite: false
            ),
            [.pin, .favorite, .archive(.thread)]
        )
        XCTAssertEqual(
            GaryxThreadRowActionPlanner.actions(
                capabilities: full,
                isPinned: true,
                isFavorite: true
            ),
            [.unpin, .unfavorite, .archive(.thread)]
        )
    }

    func testActionPlannerProducesCompleteContextMenuForAutomationTarget() {
        let automationManaged = GaryxThreadRowCapabilities(
            canOpen: true,
            canPin: true,
            canArchive: false,
            favorite: .addAndRemove,
            archiveStrategy: .none
        )
        XCTAssertEqual(
            GaryxThreadRowActionPlanner.actions(
                capabilities: automationManaged,
                isPinned: false,
                isFavorite: true
            ),
            [.pin, .unfavorite]
        )
        XCTAssertEqual(
            GaryxThreadRowActionPlanner.actions(
                capabilities: automationManaged,
                isPinned: false,
                isFavorite: false
            ),
            [.pin, .favorite]
        )
    }

    func testActionPlannerProducesCompleteContextMenuForBotEndpoint() {
        let botEndpoint = GaryxThreadRowCapabilities(
            canOpen: true,
            canPin: true,
            canArchive: true,
            favorite: .addAndRemove,
            archiveStrategy: .botEndpoint
        )

        XCTAssertEqual(
            GaryxThreadRowActionPlanner.actions(
                capabilities: botEndpoint,
                isPinned: false,
                isFavorite: false
            ),
            [.pin, .favorite, .archive(.botEndpoint)]
        )
    }

    func testActionPlannerOmitsContextMenuForUnavailableRow() {
        let unavailable = GaryxThreadRowCapabilities(
            canOpen: false,
            canPin: false,
            canArchive: false,
            favorite: .none,
            archiveStrategy: .none
        )

        XCTAssertEqual(
            GaryxThreadRowActionPlanner.actions(
                capabilities: unavailable,
                isPinned: false,
                isFavorite: false
            ),
            []
        )
    }

    func testOldGatewayPickerFallbackIsBoundedToLoadedRecentRowsAndFourFields() {
        let title = thread(id: "title", title: "Release Notes")
        let workspace = thread(id: "workspace", workspace: "/work/Orchard")
        let agent = thread(id: "agent", agent: "Reviewer")
        let preview = thread(id: "preview", preview: "Deployment finished")
        let rows = [title, workspace, agent, preview, title]

        XCTAssertEqual(
            GaryxLegacyThreadPickerFallback.rows(
                recentRows: rows,
                rawQuery: "  ORCHARD  "
            ).map(\.id),
            [workspace.id]
        )
        XCTAssertEqual(
            GaryxLegacyThreadPickerFallback.rows(
                recentRows: rows,
                rawQuery: "reviewer"
            ).map(\.id),
            [agent.id]
        )
        XCTAssertEqual(
            GaryxLegacyThreadPickerFallback.rows(
                recentRows: rows,
                rawQuery: "FINISHED"
            ).map(\.id),
            [preview.id]
        )
        XCTAssertEqual(
            GaryxLegacyThreadPickerFallback.rows(
                recentRows: rows,
                rawQuery: nil
            ).map(\.id),
            [title.id, workspace.id, agent.id, preview.id]
        )
    }

    @MainActor
    func testPresentationStoreUsesBotEntryOpenabilityAndArchiveStrategy() {
        let cache = GaryxThreadSummaryCache()
        let leaseOwner = GaryxThreadSummaryLeaseOwner(cache: cache)
        let store = GaryxThreadListStore(
            ownerId: "bot:test",
            cache: cache,
            leaseOwner: leaseOwner
        )
        let row = thread(id: "bot-thread")
        let snapshot = GaryxThreadListMembershipSnapshot(
            identity: GaryxThreadListProviderIdentity(
                kind: .botConversations(groupId: "test"),
                instanceId: 1
            ),
            orderedThreadIds: [row.id],
            isPrimed: true
        )

        store.commit(
            GaryxThreadListMembershipCommit(snapshot: snapshot, summaryWrites: [row]),
            botEntries: [
                row.id: GaryxBotConversationMembershipEntry(
                    threadId: row.id,
                    endpointKey: "bot-endpoint",
                    openable: false
                )
            ]
        )
        XCTAssertEqual(
            store.snapshot.capabilitiesById[row.id],
            GaryxThreadRowCapabilities(
                canOpen: false,
                canPin: false,
                canArchive: false,
                favorite: .none,
                archiveStrategy: .none
            )
        )

        store.commit(
            GaryxThreadListMembershipCommit(snapshot: snapshot, summaryWrites: []),
            botEntries: [
                row.id: GaryxBotConversationMembershipEntry(
                    threadId: row.id,
                    endpointKey: "bot-endpoint",
                    openable: true
                )
            ]
        )
        XCTAssertEqual(store.snapshot.capabilitiesById[row.id]?.archiveStrategy, .botEndpoint)
        XCTAssertEqual(store.snapshot.capabilitiesById[row.id]?.canArchive, true)
    }

    @MainActor
    func testPresentationStoreProjectsSharedHubMotionAndRollback() {
        let cache = GaryxThreadSummaryCache()
        let leaseOwner = GaryxThreadSummaryLeaseOwner(cache: cache)
        let store = GaryxThreadListStore(
            ownerId: "workspace:test",
            cache: cache,
            leaseOwner: leaseOwner
        )
        let row = thread(id: "pending-thread")
        let snapshot = GaryxThreadListMembershipSnapshot(
            identity: GaryxThreadListProviderIdentity(
                kind: .workspace(path: "/test"),
                instanceId: 1
            ),
            orderedThreadIds: [row.id],
            isPrimed: true
        )
        let commit = GaryxThreadListMembershipCommit(snapshot: snapshot, summaryWrites: [row])

        store.commit(
            commit,
            pendingMutations: [
                "archive": GaryxThreadMutationPendingState(
                    kind: .archive(threadId: row.id),
                    showsMotion: true,
                    ambiguous: false
                )
            ]
        )
        XCTAssertEqual(store.snapshot.motionById[row.id], .archiving)

        store.commit(
            GaryxThreadListMembershipCommit(snapshot: snapshot, summaryWrites: [])
        )
        XCTAssertTrue(store.snapshot.motionById.isEmpty)
        XCTAssertEqual(store.snapshot.rows.map(\.id), [row.id])
    }

    private func thread(
        id: String,
        title: String = "Thread",
        workspace: String? = nil,
        agent: String? = nil,
        preview: String = ""
    ) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: id,
            title: title,
            createdAt: nil,
            updatedAt: nil,
            lastMessagePreview: preview,
            workspacePath: workspace,
            messageCount: nil,
            agentId: agent,
            providerType: nil,
            recentRunId: nil,
            activeRunId: nil,
            runState: nil,
            worktreePath: nil
        )
    }
}
