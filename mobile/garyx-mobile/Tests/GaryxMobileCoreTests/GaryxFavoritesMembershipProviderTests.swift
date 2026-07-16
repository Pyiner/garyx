import Combine
import XCTest
@testable import GaryxMobileCore

@MainActor
final class GaryxFavoritesMembershipProviderTests: XCTestCase {
    func testConcurrentCapabilityConsumersShareOneProbeAndIssueOneEnhancedReplacement() async throws {
        let (owner, _, _) = makeOwner()
        let probe = FavoritesCapabilityProbeCounter()
        let machine = GaryxThreadSummaryCapabilityStateMachine(runtimeEpoch: 1) {
            await probe.run()
        }
        let resolutions = await withTaskGroup(
            of: GaryxThreadSummaryCapabilityResolution.self,
            returning: [GaryxThreadSummaryCapabilityResolution].self
        ) { group in
            for _ in 0..<10 {
                group.addTask { await machine.resolve() }
            }
            var values: [GaryxThreadSummaryCapabilityResolution] = []
            for await value in group { values.append(value) }
            return values
        }

        var effects: [GaryxFavoritesEffect] = []
        for resolution in resolutions {
            effects += owner.requestSnapshot(for: resolution).effects
        }
        let probeCount = await probe.callCount
        XCTAssertEqual(probeCount, 1)
        XCTAssertEqual(resolutions.filter(\.becameSupported).count, 1)
        XCTAssertEqual(effects.compactMap { effect -> GaryxFavoritesSnapshotTicket? in
            guard case .snapshot(let ticket) = effect else { return nil }
            return ticket
        }.count, 1)
        let ticket = try snapshotTicket(effects)
        XCTAssertEqual(ticket.requestFlavor, .enhanced)
        XCTAssertEqual(owner.state.activeSnapshotTicket, ticket)
    }

    func testSupportedTransitionCancelsLegacyAndLateCompletionHasZeroOwnerSideEffects() throws {
        let (owner, cache, leases) = makeOwner()
        var cancelled: [GaryxFavoritesSnapshotTicket] = []
        owner.onCancelSnapshot = { cancelled.append($0) }
        let legacy = try snapshotTicket(
            owner.requestSnapshot(for: resolution(.unknown, generation: 0)).effects
        )

        let transition = owner.transitionToSupported(capabilityGeneration: 1)
        let enhanced = try snapshotTicket(transition.effects)
        XCTAssertEqual(transition.cancelledTicket, legacy)
        XCTAssertEqual(cancelled, [legacy])
        XCTAssertEqual(owner.cancelledSnapshotTickets, [legacy])
        XCTAssertEqual(enhanced.requestFlavor, .enhanced)
        XCTAssertEqual(enhanced.capabilityGeneration, 1)

        let rejected = owner.completeSnapshot(
            ticket: legacy,
            snapshot: favoriteSnapshot(
                revision: 1,
                ids: ["thread::legacy"],
                recent: [thread("thread::legacy")]
            )
        )
        XCTAssertFalse(rejected.accepted)
        XCTAssertNil(cache.summary(for: "thread::legacy"))
        XCTAssertEqual(cache.count, 0)
        XCTAssertEqual(leases.activeLeaseCount, 0)
        XCTAssertTrue(owner.snapshot.orderedThreadIds.isEmpty)
        XCTAssertEqual(owner.publishCount, 0)
        XCTAssertEqual(owner.state.activeSnapshotTicket, enhanced)
    }

    func testDelayedCapabilityResolutionFromOlderRuntimeEpochCannotReplaceNewGatewayFlight() throws {
        let (owner, _, _) = makeOwner()
        let old = try snapshotTicket(
            owner.requestSnapshot(
                for: resolution(.supported, generation: 1, runtimeEpoch: 5)
            ).effects
        )
        let reconnected = owner.requestSnapshot(
            for: resolution(.unknown, generation: 2, runtimeEpoch: 6)
        )
        let current = try snapshotTicket(reconnected.effects)
        XCTAssertEqual(reconnected.cancelledTicket, old)
        XCTAssertEqual(current.requestFlavor, .legacy)
        XCTAssertEqual(owner.capabilityRuntimeEpoch, 6)

        let stale = owner.requestSnapshot(
            for: resolution(.supported, generation: 1, runtimeEpoch: 5)
        )
        XCTAssertNil(stale.cancelledTicket)
        XCTAssertTrue(stale.effects.isEmpty)
        XCTAssertEqual(owner.state.activeSnapshotTicket, current)
        XCTAssertEqual(owner.capabilityRuntimeEpoch, 6)
    }

    func testEnhancedAcceptedCommitWritesCacheSwapsLeaseAndPublishesExactlyOnceInReducerOrder() throws {
        let (owner, cache, leases) = makeOwner()
        let ticket = try snapshotTicket(
            owner.requestSnapshot(for: resolution(.supported, generation: 1)).effects
        )
        var emissions: [GaryxThreadListMembershipSnapshot] = []
        let cancellable = owner.$snapshot.dropFirst().sink { emissions.append($0) }

        let a = thread("thread::a", activitySeq: 20)
        let b = thread("thread::b", excluded: true, activitySeq: 10)
        let c = thread("thread::c")
        let decision = owner.completeSnapshot(
            ticket: ticket,
            snapshot: favoriteSnapshot(
                revision: 4,
                ids: ["thread::c", "thread::b", "thread::a"],
                recent: [a, b],
                lookup: [c, a, b],
                summariesTruncated: false
            )
        )

        XCTAssertTrue(decision.accepted)
        XCTAssertEqual(owner.snapshot.orderedThreadIds, ["thread::a", "thread::b", "thread::c"])
        XCTAssertEqual(owner.publishCount, 1)
        XCTAssertEqual(emissions.count, 1)
        XCTAssertEqual(cache.count, 3)
        XCTAssertEqual(leases.activeLeaseCount, 1)
        XCTAssertEqual(cache.pinCount(for: "thread::a"), 1)
        XCTAssertEqual(cache.pinCount(for: "thread::b"), 1)
        XCTAssertEqual(cache.pinCount(for: "thread::c"), 1)

        let capabilities = GaryxThreadRowCapabilityDeriver.capabilities(
            for: cache.summary(for: "thread::b"),
            context: .init(isFavorite: true)
        )
        XCTAssertEqual(capabilities.favorite, .removeOnly)
        withExtendedLifetime(cancellable) {}
    }

    func testEnhancedHiddenFavoriteNeverPublishesEvenWhenAnotherSourceCachedIt() throws {
        let (owner, cache, leases) = makeOwner()
        let hidden = thread("thread::hidden", title: "must stay hidden")
        cache.writeThrough([hidden])
        let ticket = try snapshotTicket(
            owner.requestSnapshot(for: resolution(.supported, generation: 3)).effects
        )
        let visible = thread("thread::visible", excluded: true)

        let decision = owner.completeSnapshot(
            ticket: ticket,
            snapshot: favoriteSnapshot(
                revision: 8,
                ids: ["thread::hidden", "thread::visible"],
                recent: [],
                lookup: [visible],
                summariesTruncated: true
            )
        )

        XCTAssertTrue(decision.accepted)
        XCTAssertEqual(owner.snapshot.orderedThreadIds, ["thread::visible"])
        XCTAssertNotNil(cache.summary(for: "thread::hidden"))
        XCTAssertEqual(cache.pinCount(for: "thread::hidden"), 0)
        XCTAssertEqual(cache.pinCount(for: "thread::visible"), 1)
        XCTAssertEqual(leases.activeLeaseCount, 1)
        XCTAssertEqual(owner.state.favoritesSummariesTruncated, true)
        XCTAssertEqual(owner.state.renderableThreadIds, ["thread::visible"])
    }

    func testFlavorAndGenerationMixupsAreRejectedWithoutCacheOrPublication() throws {
        let (owner, cache, leases) = makeOwner()
        let ticket = try snapshotTicket(
            owner.requestSnapshot(for: resolution(.supported, generation: 5)).effects
        )
        var wrongFlavor = ticket
        wrongFlavor.requestFlavor = .legacy
        var wrongGeneration = ticket
        wrongGeneration.capabilityGeneration = 4
        let response = favoriteSnapshot(
            revision: 1,
            ids: ["thread::new"],
            recent: [],
            lookup: [thread("thread::new")],
            summariesTruncated: false
        )

        XCTAssertFalse(owner.completeSnapshot(ticket: wrongFlavor, snapshot: response).accepted)
        XCTAssertFalse(owner.completeSnapshot(ticket: wrongGeneration, snapshot: response).accepted)
        XCTAssertNil(cache.summary(for: "thread::new"))
        XCTAssertEqual(leases.activeLeaseCount, 0)
        XCTAssertEqual(owner.publishCount, 0)
        XCTAssertEqual(owner.state.activeSnapshotTicket, ticket)

        XCTAssertTrue(owner.completeSnapshot(ticket: ticket, snapshot: response).accepted)
        XCTAssertEqual(owner.snapshot.orderedThreadIds, ["thread::new"])
        XCTAssertEqual(owner.publishCount, 1)
    }

    func testGatewayEpochABACompletionCannotWriteOldSummaryOrMembership() throws {
        let (owner, cache, leases) = makeOwner(scope: "gateway-a")
        let old = try snapshotTicket(
            owner.requestSnapshot(for: resolution(.unknown, generation: 0)).effects
        )
        _ = owner.replaceGatewayScope("gateway-b")
        let publicationsBeforeLateCompletion = owner.publishCount

        let decision = owner.completeSnapshot(
            ticket: old,
            snapshot: favoriteSnapshot(
                revision: 9,
                ids: ["thread::old"],
                recent: [thread("thread::old")]
            )
        )
        XCTAssertFalse(decision.accepted)
        XCTAssertNil(cache.summary(for: "thread::old"))
        XCTAssertEqual(leases.activeLeaseCount, 0)
        XCTAssertTrue(owner.snapshot.orderedThreadIds.isEmpty)
        XCTAssertEqual(owner.publishCount, publicationsBeforeLateCompletion)
    }

    func testSameScopeGatewayReconnectAdvancesReducerEpochAndRejectsOldTicket() throws {
        let (owner, cache, leases) = makeOwner(scope: "gateway")
        let old = try snapshotTicket(
            owner.requestSnapshot(for: resolution(.unknown, generation: 0)).effects
        )
        let oldEpoch = owner.state.runtimeEpoch
        _ = owner.replaceGatewayScope("gateway")
        XCTAssertEqual(owner.state.runtimeEpoch, oldEpoch + 1)

        let decision = owner.completeSnapshot(
            ticket: old,
            snapshot: favoriteSnapshot(
                revision: 3,
                ids: ["thread::old"],
                recent: [thread("thread::old")]
            )
        )
        XCTAssertFalse(decision.accepted)
        XCTAssertNil(cache.summary(for: "thread::old"))
        XCTAssertEqual(leases.activeLeaseCount, 0)
        XCTAssertTrue(owner.snapshot.orderedThreadIds.isEmpty)
    }

    func testEnhancedEnvelopeIsRequiredAndNormalRefreshRetainsFlavorGeneration() throws {
        let (owner, cache, leases) = makeOwner()
        let ticket = try snapshotTicket(
            owner.requestSnapshot(for: resolution(.supported, generation: 7)).effects
        )
        let rejected = owner.completeSnapshot(
            ticket: ticket,
            snapshot: favoriteSnapshot(
                revision: 1,
                ids: ["thread::missing-envelope"],
                recent: [thread("thread::missing-envelope")]
            )
        )
        XCTAssertFalse(rejected.accepted)
        XCTAssertTrue(owner.state.snapshotFailed)
        XCTAssertEqual(cache.count, 0)
        XCTAssertEqual(leases.activeLeaseCount, 0)
        XCTAssertEqual(owner.publishCount, 0)

        let retry = try snapshotTicket(owner.requestRefresh())
        XCTAssertEqual(retry.requestFlavor, .enhanced)
        XCTAssertEqual(retry.capabilityGeneration, 7)
    }

    func testIncarnationMismatchReplacementPreservesEnhancedFlavorAndGeneration() throws {
        let (owner, _, _) = makeOwner()
        let first = try snapshotTicket(
            owner.requestSnapshot(for: resolution(.supported, generation: 9)).effects
        )
        XCTAssertTrue(owner.completeSnapshot(
            ticket: first,
            snapshot: favoriteSnapshot(
                revision: 1,
                ids: ["thread::kept-until-barrier"],
                recent: [],
                lookup: [thread("thread::kept-until-barrier")],
                summariesTruncated: false,
                incarnation: "inc-a"
            )
        ).accepted)
        let refresh = try snapshotTicket(owner.requestRefresh())
        let rejected = owner.completeSnapshot(
            ticket: refresh,
            snapshot: favoriteSnapshot(
                revision: 0,
                ids: [],
                recent: [],
                lookup: [],
                summariesTruncated: false,
                incarnation: "inc-b"
            )
        )

        XCTAssertFalse(rejected.accepted)
        let replacement = try snapshotTicket(rejected.effects)
        XCTAssertEqual(replacement.requestFlavor, .enhanced)
        XCTAssertEqual(replacement.capabilityGeneration, 9)
        XCTAssertEqual(owner.state.snapshotRequestFlavor, .enhanced)
        XCTAssertEqual(owner.state.capabilityGeneration, 9)
        XCTAssertEqual(owner.snapshot.orderedThreadIds, ["thread::kept-until-barrier"])
    }

    private func makeOwner(
        scope: String = "gateway"
    ) -> (
        GaryxFavoritesMembershipProvider,
        GaryxThreadSummaryCache,
        GaryxThreadSummaryLeaseOwner
    ) {
        let cache = GaryxThreadSummaryCache(unpinnedCapacity: 20)
        let leases = GaryxThreadSummaryLeaseOwner(cache: cache)
        return (
            GaryxFavoritesMembershipProvider(
                gatewayScope: scope,
                cache: cache,
                leaseOwner: leases
            ),
            cache,
            leases
        )
    }

    private func resolution(
        _ state: GaryxThreadSummaryCapabilityState,
        generation: UInt64,
        runtimeEpoch: UInt64 = 0
    ) -> GaryxThreadSummaryCapabilityResolution {
        GaryxThreadSummaryCapabilityResolution(
            state: state,
            runtimeEpoch: runtimeEpoch,
            capabilityGeneration: generation,
            becameSupported: state == .supported,
            probeFailed: false
        )
    }

    private func snapshotTicket(
        _ effects: [GaryxFavoritesEffect]
    ) throws -> GaryxFavoritesSnapshotTicket {
        try XCTUnwrap(effects.compactMap { effect in
            guard case .snapshot(let ticket) = effect else { return nil }
            return ticket
        }.first)
    }

    private func favoriteSnapshot(
        revision: Int64,
        ids: [String],
        recent: [GaryxThreadSummary],
        lookup: [GaryxThreadSummary]? = nil,
        summariesTruncated: Bool? = nil,
        incarnation: String = "incarnation",
        boot: String = "boot"
    ) -> GaryxFavoriteSnapshot {
        GaryxFavoriteSnapshot(
            page: GaryxFavoritePage(
                storeIncarnationId: incarnation,
                serverBootId: boot,
                revision: revision,
                threadIds: ids
            ),
            rows: recent,
            summaryLookupRows: lookup,
            summariesTruncated: summariesTruncated
        )
    }

    private func thread(
        _ id: String,
        title: String? = nil,
        excluded: Bool = false,
        activitySeq: Int64? = nil
    ) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: id,
            title: title ?? id,
            createdAt: nil,
            updatedAt: nil,
            lastMessagePreview: "",
            workspacePath: nil,
            messageCount: 0,
            agentId: nil,
            providerType: nil,
            recentRunId: nil,
            activeRunId: nil,
            runState: nil,
            activitySeq: activitySeq,
            worktreePath: nil,
            excludeFromRecent: excluded
        )
    }
}

private actor FavoritesCapabilityProbeCounter {
    private(set) var callCount = 0

    func run() async -> GaryxThreadSummaryCapabilityProbeResult {
        callCount += 1
        await Task.yield()
        return .httpStatus(200)
    }
}
