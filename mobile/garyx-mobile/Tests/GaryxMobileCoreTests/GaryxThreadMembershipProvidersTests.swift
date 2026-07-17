import XCTest
@testable import GaryxMobileCore

final class GaryxThreadMembershipProvidersTests: XCTestCase {
    func testPickerIdentityUsesTrimmedOriginalQueryAndRejectsOldFirstPageABA() throws {
        var provider = GaryxThreadSummaryMembershipProvider(
            scope: .unscopedPicker(query: "  Straße  ")
        )
        let first = try XCTUnwrap(provider.requestRefresh())
        XCTAssertEqual(provider.identity.query, "Straße")
        XCTAssertEqual(first.query, "Straße")
        let firstInstance = provider.instanceId

        XCTAssertTrue(provider.replacePickerQuery(" STRASSE "))
        XCTAssertEqual(provider.identity.query, "STRASSE")
        XCTAssertGreaterThan(provider.instanceId, firstInstance)
        XCTAssertEqual(
            provider.completeRefresh(first, page: try summaryPage(ids: ["thread::old"])),
            .rejectedStaleInstance
        )
        XCTAssertTrue(provider.orderedThreadIds.isEmpty)

        let second = try XCTUnwrap(provider.requestRefresh())
        guard case .accepted(let commit) = provider.completeRefresh(
            second,
            page: try summaryPage(ids: ["thread::new"])
        ) else {
            return XCTFail("new query instance must accept its own first page")
        }
        XCTAssertEqual(commit.snapshot.orderedThreadIds, ["thread::new"])
    }

    func testPickerNeverClientNormalizesNFKCOrCaseFold() {
        var provider = GaryxThreadSummaryMembershipProvider(
            scope: .unscopedPicker(query: "％＿＼")
        )
        XCTAssertEqual(provider.identity.query, "％＿＼")
        let firstInstance = provider.instanceId
        XCTAssertTrue(provider.replacePickerQuery("%_\\"))
        XCTAssertEqual(provider.identity.query, "%_\\")
        XCTAssertGreaterThan(provider.instanceId, firstInstance)
    }

    @MainActor
    func testPickerOwnerQueryReplacementAtomicallyCancelsInstanceAndReleasesOnlyResultPins() throws {
        let cache = GaryxThreadSummaryCache(unpinnedCapacity: 0)
        let leases = GaryxThreadSummaryLeaseOwner(cache: cache)
        let owner = GaryxThreadPickerMembershipOwner(
            query: "old",
            cache: cache,
            leaseOwner: leases
        )
        var cancelled: [UInt64] = []
        owner.onCancelInstance = { cancelled.append($0) }
        let oldTicket = try XCTUnwrap(owner.requestRefresh())
        guard case .accepted = owner.completeRefresh(
            oldTicket,
            page: try summaryPage(ids: ["thread::old-only", "thread::selected"])
        ) else { return XCTFail("old page") }
        owner.swapSelectedTarget(thread("thread::selected"))
        XCTAssertEqual(cache.pinCount(for: "thread::selected"), 2)

        XCTAssertTrue(owner.replaceQuery("new"))
        XCTAssertEqual(cancelled, [oldTicket.instanceId])
        XCTAssertNil(cache.summary(for: "thread::old-only"))
        XCTAssertEqual(cache.pinCount(for: "thread::selected"), 1)
        XCTAssertTrue(owner.snapshot.orderedThreadIds.isEmpty)
        XCTAssertEqual(owner.identity.query, "new")
        XCTAssertEqual(
            owner.completeRefresh(
                oldTicket,
                page: try summaryPage(ids: ["thread::late"])
            ),
            .rejectedStaleInstance
        )
        XCTAssertNil(cache.summary(for: "thread::late"))

        let replacement = try XCTUnwrap(owner.requestRefresh())
        guard case .accepted = owner.completeRefresh(
            replacement,
            page: try summaryPage(ids: ["thread::selected", "thread::new"])
        ) else { return XCTFail("replacement") }
        XCTAssertEqual(cache.pinCount(for: "thread::selected"), 2)
        XCTAssertEqual(cache.pinCount(for: "thread::new"), 1)

        owner.close()
        XCTAssertEqual(cancelled, [oldTicket.instanceId, replacement.instanceId])
        XCTAssertEqual(cache.count, 0)
        XCTAssertEqual(leases.activeLeaseCount, 0)
    }

    @MainActor
    func testPickerOwnerIdentityReplacementRevokesPinsBeforeEmptySnapshotPublishes() throws {
        let cache = GaryxThreadSummaryCache(unpinnedCapacity: 0)
        let leases = GaryxThreadSummaryLeaseOwner(cache: cache)
        let owner = GaryxThreadPickerMembershipOwner(
            query: "query",
            cache: cache,
            leaseOwner: leases
        )
        var cancelled: [UInt64] = []
        owner.onCancelInstance = { cancelled.append($0) }
        let first = try XCTUnwrap(owner.requestRefresh())
        guard case .accepted = owner.completeRefresh(
            first,
            page: try summaryPage(ids: ["thread::old"])
        ) else { return XCTFail("first") }
        let mismatched = try XCTUnwrap(owner.requestRefresh())

        XCTAssertEqual(
            owner.completeRefresh(
                mismatched,
                page: try summaryPage(
                    ids: ["thread::other-store"],
                    incarnation: "restored-incarnation"
                )
            ),
            .replacementRequired
        )
        XCTAssertEqual(cancelled, [mismatched.instanceId])
        XCTAssertTrue(owner.snapshot.orderedThreadIds.isEmpty)
        XCTAssertNil(cache.summary(for: "thread::old"))
        XCTAssertNil(cache.summary(for: "thread::other-store"))
        XCTAssertEqual(leases.activeLeaseCount, 0)
    }

    func testWorkspaceProviderReusesPagerAndCarriesExactPathIntoTickets() throws {
        var provider = GaryxThreadSummaryMembershipProvider(
            scope: .workspace(path: "/workspace/project")
        )
        let refresh = try XCTUnwrap(provider.requestRefresh())
        XCTAssertEqual(refresh.workspacePath, "/workspace/project")
        XCTAssertNil(refresh.query)
        guard case .accepted = provider.completeRefresh(
            refresh,
            page: try summaryPage(ids: ["thread::a", "thread::b"], hasMore: true)
        ) else { return XCTFail("refresh") }
        let load = try XCTUnwrap(provider.requestLoadMore(trigger: .footer))
        XCTAssertEqual(load.cursor, "cursor-next")
        guard case .accepted(let commit) = provider.completeLoadMore(
            load,
            page: try summaryPage(ids: ["thread::b", "thread::c"])
        ) else { return XCTFail("load-more") }
        XCTAssertEqual(commit.snapshot.orderedThreadIds, ["thread::a", "thread::b", "thread::c"])
    }

    func testScopedProvidersExposeExplicitFailedFooterRetry() throws {
        var workspace = GaryxThreadSummaryMembershipProvider(
            scope: .workspace(path: "/workspace/project")
        )
        let workspaceHead = try XCTUnwrap(workspace.requestRefresh())
        guard case .accepted = workspace.completeRefresh(
            workspaceHead,
            page: try summaryPage(ids: ["thread::a"], hasMore: true)
        ) else { return XCTFail("workspace head") }
        let workspaceLoad = try XCTUnwrap(workspace.requestLoadMore(trigger: .footer))
        workspace.failLoadMore(workspaceLoad)
        XCTAssertNil(workspace.requestLoadMore(trigger: .footer))
        XCTAssertNotNil(workspace.retryLoadMore())

        var automation = GaryxAutomationThreadMembershipProvider(automationId: "automation::daily")
        let automationHead = try XCTUnwrap(automation.requestRefresh())
        guard case .accepted = automation.completeRefresh(
            automationHead,
            page: try automationPage(ids: ["thread::a"], offset: 0, total: 2)
        ) else { return XCTFail("automation head") }
        let automationLoad = try XCTUnwrap(automation.requestLoadMore(trigger: .footer))
        automation.failLoadMore(automationLoad)
        XCTAssertNil(automation.requestLoadMore(trigger: .footer))
        XCTAssertNotNil(automation.retryLoadMore())
    }

    func testWorkspaceIdentityChangeColdResetsAndFencesEveryOldTicket() throws {
        var provider = GaryxThreadSummaryMembershipProvider(
            scope: .workspace(path: "/workspace/project")
        )
        let first = try XCTUnwrap(provider.requestRefresh())
        guard case .accepted = provider.completeRefresh(
            first,
            page: try summaryPage(ids: ["thread::old"], hasMore: true)
        ) else { return XCTFail("first") }
        let oldInstance = provider.instanceId
        let load = try XCTUnwrap(provider.requestLoadMore(trigger: .footer))

        XCTAssertEqual(
            provider.completeLoadMore(
                load,
                page: try summaryPage(
                    ids: ["thread::new-store"],
                    incarnation: "incarnation-after-restore"
                )
            ),
            .replacementRequired
        )
        XCTAssertGreaterThan(provider.instanceId, oldInstance)
        XCTAssertTrue(provider.orderedThreadIds.isEmpty)
        XCTAssertFalse(provider.isPrimed)
        XCTAssertEqual(
            provider.completeRefresh(
                first,
                page: try summaryPage(ids: ["thread::late"])
            ),
            .rejectedStaleInstance
        )
    }

    func testBotMembershipDerivesIdsAndPointHydrationIsInstanceFenced() {
        var provider = GaryxBotConversationMembershipProvider(groupId: "telegram::main")
        let update = provider.replaceEntries(
            [
                .init(threadId: "thread::a", endpointKey: "endpoint-a"),
                .init(threadId: "thread::a", endpointKey: "duplicate"),
                .init(threadId: "thread::b", endpointKey: "endpoint-b"),
            ],
            availableSummaryIds: ["thread::a"]
        )
        XCTAssertEqual(update.commit.snapshot.orderedThreadIds, ["thread::a", "thread::b"])
        XCTAssertEqual(update.hydrationTickets.map(\.threadId), ["thread::b"])
        let ticket = update.hydrationTickets[0]
        guard case .accepted(let commit) = provider.completeHydration(
            ticket,
            summary: thread("thread::b")
        ) else { return XCTFail("hydration") }
        XCTAssertEqual(commit.summaryWrites.map(\.id), ["thread::b"])

        _ = provider.replaceEntries(
            [.init(threadId: "thread::c", endpointKey: "endpoint-c")],
            availableSummaryIds: []
        )
        XCTAssertEqual(
            provider.completeHydration(ticket, summary: thread("thread::b")),
            .rejectedStaleInstance
        )
    }

    func testAutomationProviderLoadsMoreAndUsesEmbeddedCamelAdapter() throws {
        var provider = GaryxAutomationThreadMembershipProvider(automationId: "automation::daily")
        let refresh = try XCTUnwrap(provider.requestRefresh())
        guard case .accepted(let first) = provider.completeRefresh(
            refresh,
            page: try automationPage(ids: ["thread::a", "thread::b"], offset: 0, total: 3)
        ) else { return XCTFail("first page") }
        XCTAssertEqual(first.snapshot.orderedThreadIds, ["thread::a", "thread::b"])
        XCTAssertEqual(first.summaryWrites.map(\.id), ["thread::a", "thread::b"])

        let load = try XCTUnwrap(provider.requestLoadMore(trigger: .nearTail))
        guard case .accepted(let second) = provider.completeLoadMore(
            load,
            page: try automationPage(ids: ["thread::c"], offset: 2, total: 3)
        ) else { return XCTFail("second page") }
        XCTAssertEqual(second.snapshot.orderedThreadIds, ["thread::a", "thread::b", "thread::c"])
        XCTAssertEqual(second.snapshot.footerState, .hidden)
    }

    func testAutomationEnvelopeIdentityMismatchForcesColdInstance() throws {
        var provider = GaryxAutomationThreadMembershipProvider(automationId: "automation::daily")
        let ticket = try XCTUnwrap(provider.requestRefresh())
        let oldInstance = provider.instanceId
        XCTAssertEqual(
            provider.completeRefresh(
                ticket,
                page: try automationPage(
                    ids: ["thread::wrong"],
                    offset: 0,
                    total: 1,
                    automationId: "automation::other"
                )
            ),
            .replacementRequired
        )
        XCTAssertGreaterThan(provider.instanceId, oldInstance)
        XCTAssertTrue(provider.orderedThreadIds.isEmpty)
        XCTAssertFalse(provider.isPrimed)
    }

    func testRecentWrapperPreservesExistingFeedSemantics() throws {
        var provider = GaryxRecentThreadMembershipProvider(filter: .all)
        let ticket = try XCTUnwrap(
            provider.requestRefresh(gatewayScope: "gateway", runtimeEpoch: 1)
        )
        let page = GaryxRecentThreadFeedPage(
            storeIncarnationId: "inc-a",
            serverBootId: "boot-a",
            rows: [
                GaryxRecentThreadFeedRow(id: "thread::a", activitySeq: 2),
                GaryxRecentThreadFeedRow(id: "thread::b", activitySeq: 1),
            ],
            hasMore: false,
            nextCursor: nil
        )
        guard case .accepted(let commit) = provider.completeRefresh(
            ticket,
            bundle: GaryxRecentThreadRefreshBundle(
                primaryPages: [page],
                verificationPage: page
            ),
            summaryWrites: [thread("thread::a"), thread("thread::b")]
        ) else { return XCTFail("recent wrapper") }
        XCTAssertEqual(commit.snapshot.orderedThreadIds, ["thread::a", "thread::b"])
        XCTAssertTrue(commit.snapshot.isPrimed)
    }

    func testRegistryWorkspaceLRUFourCancelsEvictedInstanceAndABAReentryIsFresh() throws {
        var registry = GaryxThreadFeedRegistry(workspaceCapacity: 4)
        let one = registry.activateWorkspace("/workspace/1")
        let two = registry.activateWorkspace("/workspace/2")
        _ = registry.activateWorkspace("/workspace/3")
        _ = registry.activateWorkspace("/workspace/4")
        let oldTicket = try XCTUnwrap(registry.issueTicket(for: .workspace("/workspace/2")))
        _ = registry.activateWorkspace("/workspace/1") // 2 becomes LRU.
        let five = registry.activateWorkspace("/workspace/5")

        XCTAssertTrue(one.coldLoad)
        XCTAssertEqual(five.evicted.map(\.key), [.workspace("/workspace/2")])
        XCTAssertEqual(five.evicted.map(\.instanceId), [two.instanceId])
        XCTAssertFalse(registry.accepts(oldTicket))

        let reentered = registry.activateWorkspace("/workspace/2")
        XCTAssertTrue(reentered.coldLoad)
        XCTAssertNotEqual(reentered.instanceId, two.instanceId)
        XCTAssertFalse(registry.accepts(oldTicket))
    }

    func testRegistryKeepsAllThreeHomeFeedsResidentAndGatewayResetFencesTheirTickets() throws {
        var registry = GaryxThreadFeedRegistry()
        let all = try XCTUnwrap(registry.issueTicket(for: .recentAll))
        let chats = try XCTUnwrap(registry.issueTicket(for: .recentChats))
        let favorites = try XCTUnwrap(registry.issueTicket(for: .favorites))
        XCTAssertTrue(registry.accepts(all))
        XCTAssertTrue(registry.accepts(chats))
        XCTAssertTrue(registry.accepts(favorites))

        registry.resetGatewayScope()
        XCTAssertFalse(registry.accepts(all))
        XCTAssertFalse(registry.accepts(chats))
        XCTAssertFalse(registry.accepts(favorites))
        XCTAssertNotNil(registry.issueTicket(for: .recentAll))
        XCTAssertNotNil(registry.issueTicket(for: .recentChats))
        XCTAssertNotNil(registry.issueTicket(for: .favorites))
    }

    @MainActor
    func testGenericStoreKeepsRecentAllPinnedSegmentAndReordersWithoutSummaryDuplication() {
        let cache = GaryxThreadSummaryCache(unpinnedCapacity: 0)
        let leases = GaryxThreadSummaryLeaseOwner(cache: cache)
        let store = GaryxThreadListStore(
            ownerId: "home-generalized",
            cache: cache,
            leaseOwner: leases,
            pinnedThreadIds: ["thread::b"]
        )
        let identity = GaryxThreadListProviderIdentity(kind: .recent(.all), instanceId: 1)
        let rows = [thread("thread::a"), thread("thread::b"), thread("thread::c")]
        let membership = GaryxThreadListMembershipSnapshot(
            identity: identity,
            orderedThreadIds: rows.map(\.id),
            isPrimed: true
        )
        XCTAssertTrue(store.commit(.init(snapshot: membership, summaryWrites: rows)))
        XCTAssertEqual(store.snapshot.pinnedThreadIds, ["thread::b"])
        XCTAssertEqual(store.snapshot.orderedThreadIds, ["thread::a", "thread::c"])
        XCTAssertEqual(store.snapshot.rows.map(\.id), ["thread::b", "thread::a", "thread::c"])

        let independentlyFetchedPin = thread("thread::pinned-outside-recent")
        store.replacePinnedOrder([independentlyFetchedPin.id, "thread::b"])
        XCTAssertTrue(store.commit(.init(
            snapshot: membership,
            summaryWrites: [independentlyFetchedPin]
        )))
        XCTAssertEqual(
            store.snapshot.pinnedThreadIds,
            ["thread::pinned-outside-recent", "thread::b"]
        )
        XCTAssertEqual(
            store.snapshot.rows.map(\.id),
            ["thread::pinned-outside-recent", "thread::b", "thread::a", "thread::c"]
        )

        XCTAssertTrue(store.commit(
            .init(snapshot: membership),
            activeRunThreadIds: ["thread::a"]
        ))
        XCTAssertFalse(store.snapshot.capabilitiesById["thread::a"]?.canArchive == true)
        XCTAssertEqual(
            store.snapshot.capabilitiesById["thread::a"]?.archiveStrategy,
            GaryxThreadArchiveStrategy.none
        )

        store.replacePinnedOrder(["thread::c", "thread::b"])
        XCTAssertTrue(store.commit(
            .init(snapshot: membership),
            activeRunThreadIds: ["thread::a"]
        ))
        XCTAssertEqual(store.snapshot.pinnedThreadIds, ["thread::c", "thread::b"])
        XCTAssertEqual(store.snapshot.rows.map(\.id), ["thread::c", "thread::b", "thread::a"])
        XCTAssertEqual(cache.count, 3)
    }

    private func summaryPage(
        ids: [String],
        hasMore: Bool = false,
        incarnation: String = "inc-a",
        boot: String = "boot-a"
    ) throws -> GaryxThreadSummariesPage {
        let rows = ids.map { id in
            """
            {"thread_id":"\(id)","title":"\(id)","workspace_dir":"/workspace/project","thread_type":"chat","provider_type":null,"agent_id":null,"created_at":null,"updated_at":null,"message_count":0,"last_user_message":null,"last_assistant_message":null,"last_message_preview":"","recent_run_id":null,"active_run_id":null,"worktree":null}
            """
        }.joined(separator: ",")
        return try JSONDecoder().decode(
            GaryxThreadSummariesPage.self,
            from: Data(
                """
                {"threads":[\(rows)],"next_cursor":\(hasMore ? "\"cursor-next\"" : "null"),"has_more":\(hasMore),"store_incarnation_id":"\(incarnation)","server_boot_id":"\(boot)"}
                """.utf8
            )
        )
    }

    private func automationPage(
        ids: [String],
        offset: Int,
        total: Int,
        automationId: String = "automation::daily"
    ) throws -> GaryxAutomationThreadsPage {
        let items = ids.enumerated().map { index, id in
            """
            {"automationId":"\(automationId)","runId":"run-\(offset + index)","threadId":"\(id)","status":"success","startedAt":"2026-07-17T00:00:00Z","thread":{"id":"\(id)","threadId":"\(id)","title":"\(id)","messageCount":0}}
            """
        }.joined(separator: ",")
        return try JSONDecoder().decode(
            GaryxAutomationThreadsPage.self,
            from: Data(
                """
                {"automationId":"\(automationId)","items":[\(items)],"count":\(ids.count),"total":\(total),"limit":2,"offset":\(offset),"hasMore":\(offset + ids.count < total)}
                """.utf8
            )
        )
    }

    private func thread(_ id: String) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: id,
            title: id,
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
}
