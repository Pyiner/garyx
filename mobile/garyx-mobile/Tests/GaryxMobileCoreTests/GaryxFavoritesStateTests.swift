import XCTest
@testable import GaryxMobileCore

final class GaryxFavoritesStateTests: XCTestCase {
    private let scope = "https://gateway.test"

    func testColdIdentityBootstrapsWithoutClearingButRecentAloneIsNotWriteReady() {
        var state = GaryxFavoritesState(gatewayScope: scope)
        let identity = state.observeStoreIdentity(
            stamp: stamp(state),
            responseStoreIncarnationId: "inc-a"
        )
        XCTAssertEqual(identity.decision, .accept)
        XCTAssertEqual(state.runtimeEpoch, 0)
        XCTAssertEqual(state.storeIncarnationId, "inc-a")
        XCTAssertNil(state.rawRevision)

        let effects = state.toggle(threadId: "thread::queued", desired: true)
        XCTAssertTrue(state.isPresented(threadId: "thread::queued"))
        XCTAssertNil(mutation(in: effects))
        XCTAssertEqual(state.intents["thread::queued"]?.phase, .active)
    }

    func testOldEpochDropsBeforeIncarnationAndCurrentMismatchClearsExactlyOnce() {
        var state = prime()
        let oldEpoch = state.runtimeEpoch
        _ = state.replaceGatewayScope("https://gateway-b.test", requestSnapshot: false)
        let old = state.observeStoreIdentity(
            stamp: GaryxStoreResponseStamp(
                gatewayScope: scope,
                runtimeEpoch: oldEpoch,
                owned: true
            ),
            responseStoreIncarnationId: "inc-a"
        )
        XCTAssertEqual(old.decision, .drop)

        _ = state.observeStoreIdentity(
            stamp: stamp(state),
            responseStoreIncarnationId: "inc-b"
        )
        let currentEpoch = state.runtimeEpoch
        _ = state.toggle(threadId: "thread::queued", desired: true)
        let changed = state.observeStoreIdentity(
            stamp: stamp(state),
            responseStoreIncarnationId: "inc-c"
        )
        XCTAssertEqual(changed.decision, .scopeClear)
        XCTAssertEqual(state.runtimeEpoch, currentEpoch + 1)
        XCTAssertNil(state.storeIncarnationId)
        XCTAssertTrue(state.intents.isEmpty)
        XCTAssertNotNil(snapshotTicket(in: changed.effects))

        let staleAgain = state.observeStoreIdentity(
            stamp: GaryxStoreResponseStamp(
                gatewayScope: state.gatewayScope,
                runtimeEpoch: currentEpoch,
                owned: true
            ),
            responseStoreIncarnationId: "inc-b"
        )
        XCTAssertEqual(staleAgain.decision, .drop)
        XCTAssertEqual(state.runtimeEpoch, currentEpoch + 1)
    }

    func testFirstSnapshotFailureKeepsQueuedIntentAndNextSuccessDrains() throws {
        var state = GaryxFavoritesState(gatewayScope: scope)
        let first = try XCTUnwrap(snapshotTicket(in: state.requestSnapshot()))
        XCTAssertNil(mutation(in: state.toggle(threadId: "thread::queued", desired: true)))
        _ = state.failSnapshot(ticket: first)
        XCTAssertNotNil(state.intents["thread::queued"])

        let second = try XCTUnwrap(snapshotTicket(in: state.requestSnapshot()))
        let effects = state.completeSnapshot(
            ticket: second,
            snapshot: snapshot(revision: 7)
        )
        let ticket = try XCTUnwrap(mutation(in: effects))
        XCTAssertTrue(ticket.target)
        XCTAssertEqual(ticket.expectedRevision, 7)
        XCTAssertEqual(ticket.expectedStoreIncarnation, "inc-a")
    }

    func testOkWithNewerReverseIntentDrainsAndConverges() throws {
        var state = prime(revision: 2)
        let first = try XCTUnwrap(mutation(in: state.toggle(threadId: "thread::a", desired: true)))
        _ = state.toggle(threadId: "thread::a", desired: false)
        var effects = state.settle(
            ticket: first,
            settlement: .ok(page(revision: 3, ids: ["thread::a"]))
        )
        let reverse = try XCTUnwrap(mutation(in: effects))
        XCTAssertFalse(reverse.target)
        XCTAssertFalse(state.isPresented(threadId: "thread::a"))
        effects = state.settle(
            ticket: reverse,
            settlement: .ok(page(revision: 4))
        )
        XCTAssertTrue(effects.isEmpty)
        XCTAssertNil(state.intents["thread::a"])
    }

    func testR11_7FenceSurvivesToggleNotSentAndGetAtExpectedRevision() throws {
        var state = prime(revision: 5)
        let put = try XCTUnwrap(mutation(in: state.toggle(threadId: "thread::a", desired: true)))
        _ = state.settle(ticket: put, settlement: .ambiguous(message: "lost"))
        XCTAssertEqual(state.unresolvedFence["thread::a"], 5)

        let remove = try XCTUnwrap(mutation(in: state.toggle(threadId: "thread::a", desired: false)))
        let delayed = state.settle(ticket: remove, settlement: .notSent(message: "not sent"))
        let timer = try XCTUnwrap(backoff(in: delayed))
        _ = state.acceptReadPage(stamp: stamp(state), page: page(revision: 5))
        XCTAssertNotNil(state.intents["thread::a"])
        XCTAssertEqual(state.unresolvedFence["thread::a"], 5)

        let retried = try XCTUnwrap(mutation(in: state.fireBackoff(timer)))
        XCTAssertFalse(retried.target)
        XCTAssertEqual(retried.expectedRevision, 5)
    }

    func testR11_8RawMismatchDoesNotBypassRetryTimer() throws {
        var state = prime(revision: 5)
        let first = try XCTUnwrap(mutation(in: state.toggle(threadId: "thread::a", desired: true)))
        let delayed = state.settle(ticket: first, settlement: .notSent(message: "not sent"))
        let timer = try XCTUnwrap(backoff(in: delayed))
        XCTAssertNil(mutation(in: state.acceptReadPage(stamp: stamp(state), page: page(revision: 6))))
        XCTAssertEqual(
            state.intents["thread::a"]?.phase,
            .retryScheduled(effectToken: timer.effectToken, cause: .notSent)
        )
        XCTAssertEqual(
            try XCTUnwrap(mutation(in: state.fireBackoff(timer))).expectedRevision,
            6
        )
    }

    func testNewerIntentEqualToRawRetiresAfterNotSentButCompensatesAmbiguity() throws {
        var provablyUnsent = prime(revision: 5)
        let first = try XCTUnwrap(mutation(in: provablyUnsent.toggle(
            threadId: "thread::a",
            desired: true
        )))
        _ = provablyUnsent.toggle(threadId: "thread::a", desired: false)
        XCTAssertTrue(provablyUnsent.settle(
            ticket: first,
            settlement: .notSent(message: "not sent")
        ).isEmpty)
        XCTAssertNil(provablyUnsent.intents["thread::a"])

        var ambiguous = prime(revision: 5)
        let unknowable = try XCTUnwrap(mutation(in: ambiguous.toggle(
            threadId: "thread::a",
            desired: true
        )))
        _ = ambiguous.toggle(threadId: "thread::a", desired: false)
        let compensation = try XCTUnwrap(mutation(in: ambiguous.settle(
            ticket: unknowable,
            settlement: .ambiguous(message: "connection lost")
        )))
        XCTAssertFalse(compensation.target)
        XCTAssertEqual(compensation.expectedRevision, 5)
        XCTAssertEqual(ambiguous.unresolvedFence["thread::a"], 5)
    }

    func testConflict404AndTerminalRejectMatrix() throws {
        var state = prime(revision: 1)
        let put = try XCTUnwrap(mutation(in: state.toggle(threadId: "thread::a", desired: true)))
        _ = state.settle(
            ticket: put,
            settlement: .definitive(
                status: 409,
                code: "revision_conflict",
                message: nil,
                page: page(revision: 2, ids: ["thread::a"])
            )
        )
        XCTAssertNil(state.intents["thread::a"])

        let remove = try XCTUnwrap(mutation(in: state.toggle(threadId: "thread::a", desired: false)))
        _ = state.settle(
            ticket: remove,
            settlement: .definitive(
                status: 404,
                code: "thread_not_found",
                message: nil,
                page: page(revision: 3)
            )
        )
        XCTAssertFalse(state.isPresented(threadId: "thread::a"))

        let rejected = try XCTUnwrap(mutation(in: state.toggle(threadId: "thread::b", desired: true)))
        _ = state.toggle(threadId: "thread::b", desired: false)
        let effects = state.settle(
            ticket: rejected,
            settlement: .definitive(
                status: 403,
                code: "forbidden",
                message: "forbidden",
                page: nil
            )
        )
        XCTAssertNil(surfaceError(in: effects))
        XCTAssertNil(mutation(in: effects))
        XCTAssertNil(state.intents["thread::b"])
    }

    func testBackoffFourTupleAndOldFlightSettleAreInert() throws {
        var state = prime()
        let first = try XCTUnwrap(mutation(in: state.toggle(threadId: "thread::a", desired: true)))
        let timer = try XCTUnwrap(backoff(in: state.settle(
            ticket: first,
            settlement: .ambiguous(message: "lost")
        )))
        let second = try XCTUnwrap(mutation(in: state.toggle(threadId: "thread::a", desired: false)))
        XCTAssertTrue(state.fireBackoff(timer).isEmpty)
        XCTAssertEqual(state.inFlight["thread::a"]?.requestToken, second.requestToken)
        XCTAssertTrue(state.settle(
            ticket: first,
            settlement: .ok(page(revision: 2, ids: ["thread::a"]))
        ).isEmpty)
        XCTAssertEqual(state.inFlight["thread::a"]?.requestToken, second.requestToken)
    }

    func testWrongIncarnationSettlementUsesThreeStepClear() throws {
        var state = prime()
        let ticket = try XCTUnwrap(mutation(in: state.toggle(threadId: "thread::a", desired: true)))
        let oldEpoch = state.runtimeEpoch
        let effects = state.settle(
            ticket: ticket,
            settlement: .definitive(
                status: 409,
                code: "wrong_incarnation",
                message: nil,
                page: page(revision: 0, incarnation: "inc-b", boot: "boot-b")
            )
        )
        XCTAssertEqual(state.runtimeEpoch, oldEpoch + 1)
        XCTAssertNil(state.storeIncarnationId)
        XCTAssertTrue(state.intents.isEmpty)
        XCTAssertNotNil(snapshotTicket(in: effects))
    }

    func testSnapshotIsAtomicAtRevisionHighWaterAndCoalescesTrailingTriggers() throws {
        var state = prime(revision: 2, ids: ["thread::a"])
        let mutationTicket = try XCTUnwrap(mutation(in: state.toggle(
            threadId: "thread::b",
            desired: true
        )))
        _ = state.settle(
            ticket: mutationTicket,
            settlement: .ok(page(revision: 4, ids: ["thread::a", "thread::b"]))
        )
        let snapshotTicket = try XCTUnwrap(snapshotTicket(in: state.requestSnapshot()))
        XCTAssertTrue(state.requestSnapshot().isEmpty)
        XCTAssertTrue(state.requestSnapshot().isEmpty)
        let effects = state.completeSnapshot(
            ticket: snapshotTicket,
            snapshot: snapshot(revision: 3, ids: ["thread::a"])
        )
        XCTAssertEqual(state.rawThreadIds, ["thread::a", "thread::b"])
        XCTAssertEqual(state.favoriteRows.map(\.id), ["thread::a"])
        XCTAssertEqual(effects.compactMap(snapshotEffect).count, 1)
        XCTAssertNotNil(state.activeSnapshotTicket)
    }

    func testPresentedRowsApplyIntentAfterCachedSnapshot() throws {
        var state = prime(revision: 1, ids: ["thread::a", "thread::b"])
        let remove = try XCTUnwrap(mutation(in: state.toggle(threadId: "thread::a", desired: false)))
        XCTAssertEqual(state.presentedRows.map(\.id), ["thread::b"])
        _ = state.settle(
            ticket: remove,
            settlement: .definitive(
                status: 400,
                code: "invalid_request",
                message: nil,
                page: nil
            )
        )
        XCTAssertEqual(state.presentedRows.map(\.id), ["thread::a", "thread::b"])
    }

    func testRetryableRejectionSchedulesAndTerminalSameGenerationSurfacesError() throws {
        var state = prime(revision: 1)
        let first = try XCTUnwrap(mutation(in: state.toggle(
            threadId: "thread::a",
            desired: true
        )))
        let delayed = state.settle(
            ticket: first,
            settlement: .definitive(
                status: 429,
                code: "rate_limited",
                message: "slow down",
                page: nil
            )
        )
        let timer = try XCTUnwrap(backoff(in: delayed))
        XCTAssertEqual(
            state.intents["thread::a"]?.phase,
            .retryScheduled(effectToken: timer.effectToken, cause: .rejected)
        )
        let retried = try XCTUnwrap(mutation(in: state.fireBackoff(timer)))
        let rejected = state.settle(
            ticket: retried,
            settlement: .definitive(
                status: 403,
                code: "forbidden",
                message: "Favorite denied",
                page: nil
            )
        )
        XCTAssertEqual(surfaceError(in: rejected), "Favorite denied")
        XCTAssertNil(state.intents["thread::a"])
        XCTAssertFalse(state.isPresented(threadId: "thread::a"))
    }

    func testRawAcceptanceResolvesAwaitVerifyAndRetryScheduledWithoutBypassingFences() throws {
        var verified = prime(revision: 5)
        let ambiguous = try XCTUnwrap(mutation(in: verified.toggle(
            threadId: "thread::a",
            desired: true
        )))
        _ = verified.settle(ticket: ambiguous, settlement: .ambiguous(message: "lost"))
        let verifyEffects = verified.acceptReadPage(
            stamp: stamp(verified),
            page: page(revision: 6, ids: ["thread::a"])
        )
        XCTAssertTrue(verifyEffects.isEmpty)
        XCTAssertNil(verified.intents["thread::a"])
        XCTAssertNil(verified.unresolvedFence["thread::a"])

        var retried = prime(revision: 5)
        let notSent = try XCTUnwrap(mutation(in: retried.toggle(
            threadId: "thread::a",
            desired: true
        )))
        let delayed = retried.settle(
            ticket: notSent,
            settlement: .notSent(message: "not sent")
        )
        let timer = try XCTUnwrap(backoff(in: delayed))
        XCTAssertTrue(retried.acceptReadPage(
            stamp: stamp(retried),
            page: page(revision: 6, ids: ["thread::a"])
        ).isEmpty)
        XCTAssertNil(retried.intents["thread::a"])
        XCTAssertTrue(retried.fireBackoff(timer).isEmpty)
    }

    func testDifferentThreadFlightsStayIsolatedAndAllBackoffStampFieldsFenceEffects() throws {
        var state = prime(revision: 1)
        let first = try XCTUnwrap(mutation(in: state.toggle(threadId: "thread::a", desired: true)))
        let second = try XCTUnwrap(mutation(in: state.toggle(threadId: "thread::b", desired: true)))
        _ = state.settle(
            ticket: first,
            settlement: .ok(page(revision: 2, ids: ["thread::a"]))
        )
        XCTAssertEqual(state.inFlight["thread::b"]?.requestToken, second.requestToken)

        let delayed = state.settle(
            ticket: second,
            settlement: .notSent(message: "not sent")
        )
        let timer = try XCTUnwrap(backoff(in: delayed))
        let baseline = state
        var wrongScope = timer
        wrongScope.gatewayScope = "https://other.test"
        var wrongEpoch = timer
        wrongEpoch.runtimeEpoch &+= 1
        var wrongGeneration = timer
        wrongGeneration.generation &+= 1
        var wrongEffect = timer
        wrongEffect.effectToken &+= 1
        for stamp in [wrongScope, wrongEpoch, wrongGeneration, wrongEffect] {
            XCTAssertTrue(state.fireBackoff(stamp).isEmpty)
            XCTAssertEqual(state, baseline)
        }
        XCTAssertNotNil(mutation(in: state.fireBackoff(timer)))
    }

    func testBootChangeRequestsSnapshotAndExternalDeleteCannotReviveCachedRow() throws {
        var state = prime(revision: 1, ids: ["thread::a"])
        let put = try XCTUnwrap(mutation(in: state.toggle(
            threadId: "thread::b",
            desired: true
        )))
        let bootEffects = state.settle(
            ticket: put,
            settlement: .ok(page(
                revision: 2,
                ids: ["thread::a", "thread::b"],
                boot: "boot-b"
            ))
        )
        XCTAssertNotNil(snapshotTicket(in: bootEffects))
        XCTAssertEqual(state.presentedRows.map(\.id), ["thread::a"])

        _ = state.acceptReadPage(
            stamp: stamp(state),
            page: page(revision: 3, ids: ["thread::b"], boot: "boot-b")
        )
        XCTAssertTrue(state.presentedRows.isEmpty)
        XCTAssertFalse(
            state.isPresented(threadId: "thread::a"),
            "a cached snapshot row must not revive an externally deleted favorite"
        )
    }

    private func prime(revision: Int64 = 1, ids: [String] = []) -> GaryxFavoritesState {
        var state = GaryxFavoritesState(gatewayScope: scope)
        let ticket = snapshotTicket(in: state.requestSnapshot())!
        _ = state.completeSnapshot(
            ticket: ticket,
            snapshot: snapshot(revision: revision, ids: ids)
        )
        return state
    }

    private func stamp(_ state: GaryxFavoritesState) -> GaryxStoreResponseStamp {
        GaryxStoreResponseStamp(
            gatewayScope: state.gatewayScope,
            runtimeEpoch: state.runtimeEpoch,
            owned: true
        )
    }

    private func page(
        revision: Int64,
        ids: [String] = [],
        incarnation: String = "inc-a",
        boot: String = "boot-a"
    ) -> GaryxFavoritePage {
        GaryxFavoritePage(
            storeIncarnationId: incarnation,
            serverBootId: boot,
            revision: revision,
            threadIds: ids
        )
    }

    private func snapshot(revision: Int64, ids: [String] = []) -> GaryxFavoriteSnapshot {
        GaryxFavoriteSnapshot(
            page: page(revision: revision, ids: ids),
            rows: ids.map(thread)
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
            activitySeq: 1,
            worktreePath: nil
        )
    }

    private func mutation(in effects: [GaryxFavoritesEffect]) -> GaryxFavoriteMutationTicket? {
        effects.compactMap { effect in
            guard case .mutate(let ticket) = effect else { return nil }
            return ticket
        }.first
    }

    private func snapshotTicket(in effects: [GaryxFavoritesEffect]) -> GaryxFavoritesSnapshotTicket? {
        effects.compactMap(snapshotEffect).first
    }

    private func snapshotEffect(_ effect: GaryxFavoritesEffect) -> GaryxFavoritesSnapshotTicket? {
        guard case .snapshot(let ticket) = effect else { return nil }
        return ticket
    }

    private func backoff(in effects: [GaryxFavoritesEffect]) -> GaryxFavoriteBackoffStamp? {
        effects.compactMap { effect in
            guard case .backoff(let stamp, _) = effect else { return nil }
            return stamp
        }.first
    }

    private func surfaceError(in effects: [GaryxFavoritesEffect]) -> String? {
        effects.compactMap { effect in
            guard case .surfaceError(_, let message) = effect else { return nil }
            return message
        }.first
    }
}
