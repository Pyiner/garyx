import XCTest
@testable import GaryxMobileCore

final class GaryxThreadMutationHubTests: XCTestCase {
    func testFourPhasesStayConsistentAcrossResidentStores() throws {
        var hub = GaryxThreadMutationHub(gatewayRuntimeEpoch: 7)
        hub.registerStore(storeId: "recent:all", instanceId: 11, orderedThreadIds: ["target", "keep"])
        hub.registerStore(storeId: "workspace:/workspace", instanceId: 12, orderedThreadIds: ["target", "other"])

        XCTAssertTrue(hub.began(
            mutationId: "archive-1",
            kind: .archive(threadId: "target"),
            gatewayRuntimeEpoch: 7
        ))
        for storeId in ["recent:all", "workspace:/workspace"] {
            let pending = try XCTUnwrap(hub.residents[storeId]?.pending["archive-1"])
            XCTAssertTrue(pending.showsMotion)
            XCTAssertFalse(pending.ambiguous)
        }

        let authority = GaryxThreadMutationAuthority(
            membership: .remove(threadId: "target"),
            summary: nil,
            favoriteRevision: 41
        )
        XCTAssertTrue(hub.committed(
            mutationId: "archive-1",
            gatewayRuntimeEpoch: 7,
            authority: authority
        ))
        XCTAssertEqual(hub.transactions["archive-1"]?.phase, .committed)
        XCTAssertEqual(hub.transactions["archive-1"]?.authority, authority)
        XCTAssertEqual(hub.residents["recent:all"]?.orderedThreadIds, ["keep"])
        XCTAssertEqual(hub.residents["workspace:/workspace"]?.orderedThreadIds, ["other"])
        XCTAssertTrue(hub.residents.values.allSatisfy { $0.pending.isEmpty })

        XCTAssertTrue(hub.began(
            mutationId: "pin-1",
            kind: .pin(threadId: "keep", pinned: true),
            gatewayRuntimeEpoch: 7
        ))
        XCTAssertTrue(hub.rolledBack(
            mutationId: "pin-1",
            gatewayRuntimeEpoch: 7,
            message: "conflict"
        ))
        XCTAssertEqual(hub.transactions["pin-1"]?.phase, .rolledBack(message: "conflict"))
        XCTAssertTrue(hub.residents.values.allSatisfy { $0.pending.isEmpty })

        XCTAssertFalse(hub.began(
            mutationId: "wrong-epoch",
            kind: .insert(threadId: "new"),
            gatewayRuntimeEpoch: 6
        ))
    }

    func testAmbiguousArchiveKeepsSnapshotCancelsMotionAndAuthoritativeReplacementCommitsPerInstance() throws {
        var hub = GaryxThreadMutationHub(gatewayRuntimeEpoch: 3)
        hub.registerStore(storeId: "recent:all", instanceId: 100, orderedThreadIds: ["target", "keep"])
        hub.registerStore(storeId: "workspace:/workspace", instanceId: 101, orderedThreadIds: ["target", "other"])
        XCTAssertTrue(hub.began(
            mutationId: "archive",
            kind: .archive(threadId: "target"),
            gatewayRuntimeEpoch: 3
        ))

        let tickets = hub.ambiguous(mutationId: "archive", gatewayRuntimeEpoch: 3)
        XCTAssertEqual(tickets.count, 2)
        for resident in hub.residents.values {
            XCTAssertEqual(resident.orderedThreadIds.first, "target")
            let pending = try XCTUnwrap(resident.pending["archive"])
            XCTAssertTrue(pending.ambiguous)
            XCTAssertFalse(pending.showsMotion)
            XCTAssertEqual(resident.barrier?.state, .pending)
        }

        for ticket in tickets {
            let ids = ticket.storeId == "recent:all" ? ["keep"] : ["other"]
            XCTAssertEqual(
                hub.completeReconstruction(ticket, outcome: .authoritative(orderedThreadIds: ids)),
                .accepted
            )
        }
        XCTAssertEqual(hub.residents["recent:all"]?.orderedThreadIds, ["keep"])
        XCTAssertEqual(hub.residents["workspace:/workspace"]?.orderedThreadIds, ["other"])
        XCTAssertTrue(hub.residents.values.allSatisfy { $0.pending.isEmpty && $0.barrier == nil })
    }

    func testQueuedBarrierCannotBeConsumedByOlderTicketAndFailureIsStickyUntilRetry() throws {
        var hub = GaryxThreadMutationHub(gatewayRuntimeEpoch: 9)
        hub.registerStore(storeId: "recent:all", instanceId: 50, orderedThreadIds: ["one", "two"])

        XCTAssertTrue(hub.began(
            mutationId: "first",
            kind: .archive(threadId: "one"),
            gatewayRuntimeEpoch: 9
        ))
        let firstTicket = try XCTUnwrap(
            hub.ambiguous(mutationId: "first", gatewayRuntimeEpoch: 9).first
        )
        XCTAssertTrue(hub.began(
            mutationId: "second",
            kind: .archive(threadId: "two"),
            gatewayRuntimeEpoch: 9
        ))
        let replacement = try XCTUnwrap(
            hub.ambiguous(mutationId: "second", gatewayRuntimeEpoch: 9).first
        )

        XCTAssertEqual(
            hub.completeReconstruction(firstTicket, outcome: .authoritative(orderedThreadIds: [])),
            .rejectedStaleTicket
        )
        XCTAssertEqual(hub.residents["recent:all"]?.orderedThreadIds, ["one", "two"])
        XCTAssertEqual(hub.residents["recent:all"]?.barrier?.coveredMutationIds, ["first", "second"])

        XCTAssertEqual(
            hub.completeReconstruction(replacement, outcome: .failed(message: "offline")),
            .accepted
        )
        XCTAssertEqual(hub.residents["recent:all"]?.barrier?.state, .failed(message: "offline"))
        let pendingIds = Set(hub.residents["recent:all"].map { Array($0.pending.keys) } ?? [])
        XCTAssertEqual(pendingIds, ["first", "second"])
        XCTAssertEqual(
            hub.completeReconstruction(
                replacement,
                outcome: .authoritative(orderedThreadIds: [])
            ),
            .rejectedStaleTicket,
            "a failed generation stays sticky until an explicit retry"
        )

        let retry = try XCTUnwrap(hub.retryReconstruction(storeId: "recent:all"))
        XCTAssertGreaterThan(retry.generation, replacement.generation)
        XCTAssertEqual(hub.residents["recent:all"]?.barrier?.state, .pending)
        XCTAssertEqual(
            hub.completeReconstruction(replacement, outcome: .authoritative(orderedThreadIds: [])),
            .rejectedStaleTicket
        )
        XCTAssertEqual(
            hub.completeReconstruction(retry, outcome: .authoritative(orderedThreadIds: ["two"])),
            .accepted
        )
        XCTAssertEqual(hub.residents["recent:all"]?.orderedThreadIds, ["two"])
    }

    func testEvictionCompletesBarrierAndColdReentryRejectsOldTicket() throws {
        var hub = GaryxThreadMutationHub(gatewayRuntimeEpoch: 1)
        hub.registerStore(storeId: "workspace:/old", instanceId: 4, orderedThreadIds: ["target"])
        XCTAssertTrue(hub.began(
            mutationId: "archive",
            kind: .archive(threadId: "target"),
            gatewayRuntimeEpoch: 1
        ))
        let oldTicket = try XCTUnwrap(
            hub.ambiguous(mutationId: "archive", gatewayRuntimeEpoch: 1).first
        )

        hub.evictStore(storeId: "workspace:/old", instanceId: 4)
        hub.registerStore(storeId: "workspace:/old", instanceId: 5, orderedThreadIds: [])
        XCTAssertEqual(
            hub.completeReconstruction(oldTicket, outcome: .authoritative(orderedThreadIds: ["target"])),
            .rejectedStaleTicket
        )
        XCTAssertEqual(hub.residents["workspace:/old"]?.orderedThreadIds, [])
        XCTAssertTrue(hub.residents["workspace:/old"]?.pending.isEmpty == true)
        XCTAssertNil(hub.residents["workspace:/old"]?.barrier)
    }

    func testGatewayEpochResetRejectsABAAndFavoritesUsesTerminalFanOutOnly() {
        var hub = GaryxThreadMutationHub(gatewayRuntimeEpoch: 20)
        hub.registerStore(storeId: "recent:all", instanceId: 1, orderedThreadIds: ["old"])
        XCTAssertTrue(hub.began(
            mutationId: "rename",
            kind: .rename(threadId: "old"),
            gatewayRuntimeEpoch: 20
        ))
        hub.resetGatewayScope(runtimeEpoch: 21)
        hub.registerStore(storeId: "recent:all", instanceId: 2, orderedThreadIds: ["new"])
        XCTAssertFalse(hub.committed(
            mutationId: "rename",
            gatewayRuntimeEpoch: 20,
            authority: .init(membership: .upsertAtHead(threadId: "old"))
        ))

        hub.fanOutFavoritesCommitted(
            mutationId: "favorite",
            threadId: "fav",
            favorited: true,
            authority: .init(membership: .upsertAtHead(threadId: "fav"))
        )
        XCTAssertEqual(hub.transactions["favorite"]?.phase, .committed)
        XCTAssertEqual(hub.residents["recent:all"]?.orderedThreadIds, ["fav", "new"])
        XCTAssertTrue(hub.residents["recent:all"]?.pending.isEmpty == true)
        XCTAssertNil(hub.residents["recent:all"]?.barrier)
    }

    func testInsertAffectsOnlyRecentAndMatchingResidentWorkspace() {
        var hub = GaryxThreadMutationHub(gatewayRuntimeEpoch: 4)
        hub.registerStore(storeId: "home", instanceId: 1, orderedThreadIds: ["recent"])
        hub.registerStore(storeId: "recent:all", instanceId: 2, orderedThreadIds: ["recent"])
        hub.registerStore(
            storeId: "workspace:/workspace",
            instanceId: 3,
            orderedThreadIds: ["workspace-old"]
        )
        hub.registerStore(
            storeId: "workspace:/other",
            instanceId: 4,
            orderedThreadIds: ["other-old"]
        )
        hub.registerStore(storeId: "bot:bot-1", instanceId: 5, orderedThreadIds: ["bot-old"])
        hub.registerStore(
            storeId: "automation:auto-1",
            instanceId: 6,
            orderedThreadIds: ["automation-old"]
        )

        let affected = hub.residentStoreIdsAffectedByInsert(workspacePath: " /workspace ")
        XCTAssertEqual(affected, ["home", "recent:all", "workspace:/workspace"])
        XCTAssertTrue(hub.began(
            mutationId: "insert",
            kind: .insert(threadId: "new"),
            gatewayRuntimeEpoch: 4,
            affectedStoreIds: affected
        ))
        XCTAssertTrue(hub.committed(
            mutationId: "insert",
            gatewayRuntimeEpoch: 4,
            authority: .init(membership: .upsertAtHead(threadId: "new"))
        ))

        XCTAssertEqual(hub.residents["home"]?.orderedThreadIds, ["new", "recent"])
        XCTAssertEqual(hub.residents["recent:all"]?.orderedThreadIds, ["new", "recent"])
        XCTAssertEqual(
            hub.residents["workspace:/workspace"]?.orderedThreadIds,
            ["new", "workspace-old"]
        )
        XCTAssertEqual(hub.residents["workspace:/other"]?.orderedThreadIds, ["other-old"])
        XCTAssertEqual(hub.residents["bot:bot-1"]?.orderedThreadIds, ["bot-old"])
        XCTAssertEqual(
            hub.residents["automation:auto-1"]?.orderedThreadIds,
            ["automation-old"]
        )
    }
}
