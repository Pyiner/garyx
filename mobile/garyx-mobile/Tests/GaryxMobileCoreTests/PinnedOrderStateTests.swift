import XCTest
@testable import GaryxMobileCore

final class PinnedOrderStateTests: XCTestCase {
    private let gateway = "gateway-a"

    func testPreviewOnlyMovesAndCancelAfterMultipleMovesMutatesNothing() throws {
        var state = makeState(["a", "b", "c"], revision: 4)
        let epoch = state.epoch
        _ = state.beginDrag()
        _ = state.previewDrag(order: ["b", "a", "c"])
        _ = state.previewDrag(order: ["c", "b", "a"])

        let update = state.cancelDrag()

        XCTAssertEqual(state.presentedOrder, ["a", "b", "c"])
        XCTAssertEqual(state.epoch, epoch)
        XCTAssertNil(state.outbox)
        XCTAssertTrue(sends(update).isEmpty)
        XCTAssertFalse(update.effects.contains(.noteLocalMutation))
    }

    func testAcceptedDropCommitsOnceAndStartsOneFlight() throws {
        var state = makeState(["a", "b"], revision: 4)
        _ = state.beginDrag()
        _ = state.previewDrag(order: ["b", "a"])

        let update = state.acceptDrop()

        XCTAssertEqual(state.desiredOrder, ["b", "a"])
        XCTAssertEqual(state.epoch, 1)
        XCTAssertEqual(sends(update).count, 1)
        XCTAssertEqual(sends(update).first?.threadIds, ["b", "a"])
        XCTAssertEqual(sends(update).first?.expectedRevision, 4)
        XCTAssertEqual(update.effects.filter { $0 == .noteLocalMutation }.count, 1)
    }

    func testHomeListStoreOwnsPinnedOrderReducerDomain() throws {
        let store = GaryxHomeThreadListStore()
        _ = store.updatePinnedOrderState { state in
            state.switchGateway(to: gateway)
        }
        _ = store.updatePinnedOrderState { state in
            state.receivePage(page(["a", "b"], 7), stamp: state.requestStamp())
        }
        _ = store.updatePinnedOrderState { state in
            state.beginDrag()
        }
        _ = store.updatePinnedOrderState { state in
            state.previewDrag(order: ["b", "a"])
        }

        let update = store.updatePinnedOrderState { state in
            state.acceptDrop()
        }

        XCTAssertEqual(store.pinnedOrderState.desiredOrder, ["b", "a"])
        XCTAssertEqual(try XCTUnwrap(sends(update).first).expectedRevision, 7)
    }

    func testHomeListStoreExposesNonBlockingPendingSyncStatus() throws {
        let store = GaryxHomeThreadListStore()
        _ = store.updatePinnedOrderState { state in
            state.switchGateway(to: gateway)
        }
        _ = store.updatePinnedOrderState { state in
            state.receivePage(page(["a", "b"], 7), stamp: state.requestStamp())
        }
        _ = store.updatePinnedOrderState { state in state.beginDrag() }
        _ = store.updatePinnedOrderState { state in
            state.previewDrag(order: ["b", "a"])
        }
        let drop = store.updatePinnedOrderState { state in state.acceptDrop() }
        let request = try XCTUnwrap(sends(drop).first)

        _ = store.updatePinnedOrderState { state in
            state.failReorder(request, failure: .permanent(statusCode: 405))
        }

        XCTAssertEqual(store.pinnedOrderSyncStatusLabel, "Sync pending")
        _ = store.updatePinnedOrderState { state in state.resumePausedSync() }
        XCTAssertNil(store.pinnedOrderSyncStatusLabel)
    }

    func testDragBufferKeepsHighestAcceptedProjectionForCancel() throws {
        var state = makeState(["a", "b"], revision: 10)
        _ = state.beginDrag()
        let stamp = state.requestStamp()

        let newer = state.receivePage(page(["c", "a", "b"], 12), stamp: stamp)
        let delayed = state.receivePage(page(["a", "b"], 11), stamp: stamp)

        XCTAssertEqual(newer.acceptance, .authoritative)
        XCTAssertEqual(delayed.acceptance, .discardedBelowFloor)
        XCTAssertEqual(state.presentedOrder, ["a", "b"])
        let cancel = state.cancelDrag()
        XCTAssertEqual(state.presentedOrder, ["c", "a", "b"])
        XCTAssertEqual(publications(cancel), [["c", "a", "b"]])
    }

    func testDragBufferKeepsHighestAcceptedProjectionUnderDropOverlay() throws {
        var state = makeState(["a", "b"], revision: 10)
        _ = state.beginDrag()
        _ = state.previewDrag(order: ["b", "a"])
        let stamp = state.requestStamp()
        _ = state.receivePage(page(["c", "a", "b"], 12), stamp: stamp)
        let delayed = state.receivePage(page(["a", "b"], 11), stamp: stamp)

        let drop = state.acceptDrop()

        XCTAssertEqual(delayed.acceptance, .discardedBelowFloor)
        XCTAssertEqual(state.presentedOrder, ["c", "b", "a"])
        XCTAssertEqual(sends(drop).first?.threadIds, ["c", "b", "a"])
        let currentPoll = state.requestStamp()
        _ = state.receivePage(page(["c", "a", "b"], 12), stamp: currentPoll)
        XCTAssertEqual(state.presentedOrder, ["c", "b", "a"])
    }

    func testStaleGetIssuedAfterDropCannotRevertAfterAckRaisesFloor() throws {
        var state = makeState(["a", "b"], revision: 10)
        let request = try beginDrop(&state, order: ["b", "a"])
        let staleGet = state.requestStamp()

        _ = state.completeReorder(request, page: page(["b", "a"], 12))
        let stale = state.receivePage(page(["a", "b"], 11), stamp: staleGet)

        XCTAssertEqual(stale.acceptance, .discardedBelowFloor)
        XCTAssertEqual(state.presentedOrder, ["b", "a"])
        XCTAssertNil(state.outbox)
    }

    func testRevisionDescendingPinWritebacksDiscardOldPageCompletely() throws {
        var state = makeState(["a"], revision: 10)
        let pinB = try XCTUnwrap(state.beginMembershipChange(threadId: "b", pinned: true).membershipRequest)
        let pinC = try XCTUnwrap(state.beginMembershipChange(threadId: "c", pinned: true).membershipRequest)

        _ = state.completeMembership(pinC, page: page(["c", "b", "a"], 12))
        let old = state.completeMembership(pinB, page: page(["b", "a"], 11))

        XCTAssertEqual(old.acceptance, .discardedBelowFloor)
        XCTAssertEqual(state.highestObservedRevision, 12)
        XCTAssertEqual(state.presentedOrder, ["c", "b", "a"])
    }

    func testSettleAdvancesEpochSoUnsettledWindowPageOnlyMerges() throws {
        var state = makeState(["a", "b"], revision: 10)
        let reorder = try beginDrop(&state, order: ["b", "a"])
        let unsettledWindowGet = state.requestStamp()
        _ = state.completeReorder(reorder, page: page(["b", "a"], 12))
        let settledEpoch = state.epoch

        let oldWindow = state.receivePage(
            page(["a", "b"], 13),
            stamp: unsettledWindowGet
        )

        XCTAssertEqual(oldWindow.acceptance, .merged)
        XCTAssertEqual(state.presentedOrder, ["b", "a"])
        XCTAssertGreaterThan(settledEpoch, unsettledWindowGet.epoch)
        let current = state.receivePage(page(["a", "b"], 13), stamp: state.requestStamp())
        XCTAssertEqual(current.acceptance, .authoritative)
        XCTAssertEqual(state.presentedOrder, ["a", "b"])
    }

    func testBelowFloor200EndsFlightAndResendsWithFreshFloor() throws {
        var state = makeState(["a", "b"], revision: 10)
        let first = try beginDrop(&state, order: ["b", "a"])
        _ = state.receivePage(page(["a", "b"], 12), stamp: state.requestStamp())

        let completion = state.completeReorder(first, page: page(["b", "a"], 11))
        let second = try XCTUnwrap(sends(completion).first)

        XCTAssertEqual(completion.acceptance, .discardedBelowFloor)
        XCTAssertEqual(second.expectedRevision, 12)
        XCTAssertEqual(second.threadIds, ["b", "a"])
        _ = state.completeReorder(second, page: page(["b", "a"], 13))
        XCTAssertNil(state.outbox)
    }

    func testBelowFloor409UsesFloorInsteadOfReturnedRevision() throws {
        var state = makeState(["a", "b"], revision: 10)
        let first = try beginDrop(&state, order: ["b", "a"])
        _ = state.receivePage(page(["a", "b"], 14), stamp: state.requestStamp())

        let conflict = state.completeReorder(first, page: page(["a", "b"], 11))

        XCTAssertEqual(conflict.acceptance, .discardedBelowFloor)
        XCTAssertEqual(sends(conflict).map(\.expectedRevision), [14])
    }

    func testHighRevisionOppositePinWinsWhenLocalAckIsBelowFloor() throws {
        var state = makeState(["a"], revision: 10)
        let pin = try XCTUnwrap(
            state.beginMembershipChange(threadId: "b", pinned: true).membershipRequest
        )
        _ = state.receivePage(page(["a"], 12), stamp: state.requestStamp())
        XCTAssertEqual(state.presentedOrder, ["b", "a"])

        let lowAck = state.completeMembership(pin, page: page(["b", "a"], 11))

        XCTAssertEqual(lowAck.acceptance, .discardedBelowFloor)
        XCTAssertEqual(state.presentedOrder, ["a"])
        XCTAssertEqual(state.liveMembershipIntentCount, 0)
    }

    func testHighRevisionOppositeUnpinWinsWhenLocalAckIsBelowFloor() throws {
        var state = makeState(["a", "b"], revision: 10)
        let unpin = try XCTUnwrap(
            state.beginMembershipChange(threadId: "b", pinned: false).membershipRequest
        )
        _ = state.receivePage(page(["b", "a"], 12), stamp: state.requestStamp())
        XCTAssertEqual(state.presentedOrder, ["a"])

        let lowAck = state.completeMembership(unpin, page: page(["a"], 11))

        XCTAssertEqual(lowAck.acceptance, .discardedBelowFloor)
        XCTAssertEqual(state.presentedOrder, ["b", "a"])
        XCTAssertEqual(state.liveMembershipIntentCount, 0)
    }

    func testGatewaySwitchResetsFloorAndDropsLateOldIdentityResponse() {
        var state = makeState(["a"], revision: 100)
        let oldStamp = state.requestStamp()
        let switched = state.switchGateway(to: "gateway-b")

        let fresh = state.receivePage(
            page(["new"], 0),
            stamp: GaryxPinnedOrderRequestStamp(gatewayIdentity: "gateway-b", epoch: 0)
        )
        let late = state.receivePage(page(["old"], 101), stamp: oldStamp)

        XCTAssertEqual(state.highestObservedRevision, 0)
        XCTAssertEqual(fresh.acceptance, .authoritative)
        XCTAssertFalse(late.identityAccepted)
        XCTAssertEqual(state.presentedOrder, ["new"])
        XCTAssertTrue(switched.effects.contains(.persist(nil, gatewayIdentity: gateway)))
    }

    func testAckSettlePublishesNoRowOrderDelta() throws {
        var state = makeState(["a", "b"], revision: 10)
        let request = try beginDrop(&state, order: ["b", "a"])

        let ack = state.completeReorder(request, page: page(["b", "a"], 11))

        XCTAssertNil(state.outbox)
        XCTAssertTrue(publications(ack).isEmpty)
        XCTAssertTrue(sends(ack).isEmpty)
    }

    func testConflictMergesRemotePinThenResendsFullOrderAndSettles() throws {
        var state = makeState(["a", "b"], revision: 10)
        let first = try beginDrop(&state, order: ["b", "a"])

        let conflict = state.completeReorder(first, page: page(["c", "a", "b"], 11))
        let second = try XCTUnwrap(sends(conflict).first)

        XCTAssertEqual(second.threadIds, ["c", "b", "a"])
        XCTAssertEqual(second.expectedRevision, 11)
        _ = state.completeReorder(second, page: page(["c", "b", "a"], 12))
        XCTAssertNil(state.outbox)
        XCTAssertEqual(state.presentedOrder, ["c", "b", "a"])
    }

    func testConflictPageAlreadyEqualsDesiredSettlesWithoutAnotherPut() throws {
        var state = makeState(["a", "b"], revision: 10)
        let request = try beginDrop(&state, order: ["b", "a"])

        let conflict = state.completeReorder(request, page: page(["b", "a"], 11))

        XCTAssertNil(state.outbox)
        XCTAssertTrue(sends(conflict).isEmpty)
    }

    func testPollDuringFlightKeepsLocalOrderAndRemotePinJoinsFollowup() throws {
        var state = makeState(["a", "b"], revision: 10)
        let first = try beginDrop(&state, order: ["b", "a"])
        let poll = state.receivePage(page(["c", "a", "b"], 11), stamp: state.requestStamp())

        XCTAssertEqual(poll.acceptance, .merged)
        XCTAssertEqual(state.presentedOrder, ["c", "b", "a"])
        XCTAssertTrue(sends(poll).isEmpty)
        XCTAssertEqual(state.pendingSync, .coalescedBehindFlight)

        let completion = state.completeReorder(first, page: page(["a", "b"], 10))
        XCTAssertEqual(sends(completion).first?.threadIds, ["c", "b", "a"])
        XCTAssertEqual(sends(completion).first?.expectedRevision, 11)
    }

    func testDelayedUnpinGatesResendUntilMembershipResponse() throws {
        var state = makeState(["a", "b", "c"], revision: 10)
        let first = try beginDrop(&state, order: ["c", "b", "a"])
        let unpin = try XCTUnwrap(
            state.beginMembershipChange(threadId: "a", pinned: false).membershipRequest
        )

        let conflict = state.completeReorder(first, page: page(["a", "b", "c"], 11))
        XCTAssertTrue(sends(conflict).isEmpty)
        XCTAssertEqual(state.pendingSync, .waitingForMembership)

        let membership = state.completeMembership(unpin, page: page(["b", "c"], 12))
        XCTAssertEqual(sends(membership).count, 1)
        XCTAssertEqual(sends(membership).first?.threadIds, ["c", "b"])
        XCTAssertEqual(sends(membership).first?.expectedRevision, 12)
    }

    func testDelayedPinNeverDispatchesUnknownIdBeforePinCompletes() throws {
        var state = makeState(["a", "b"], revision: 10)
        let first = try beginDrop(&state, order: ["b", "a"])
        let pin = try XCTUnwrap(
            state.beginMembershipChange(threadId: "c", pinned: true).membershipRequest
        )

        let oldFlight = state.completeReorder(first, page: page(["a", "b"], 11))
        XCTAssertTrue(sends(oldFlight).isEmpty)
        XCTAssertEqual(state.pendingSync, .waitingForMembership)

        let membership = state.completeMembership(pin, page: page(["c", "a", "b"], 12))
        XCTAssertEqual(sends(membership).first?.threadIds, ["c", "b", "a"])
    }

    func testMembershipFailureRollbackWakesGateOnce() throws {
        var state = makeState(["a", "b", "c"], revision: 10)
        let first = try beginDrop(&state, order: ["c", "b", "a"])
        let unpin = try XCTUnwrap(
            state.beginMembershipChange(threadId: "a", pinned: false).membershipRequest
        )
        _ = state.completeReorder(first, page: page(["a", "b", "c"], 11))

        let rollback = state.failMembership(unpin)

        XCTAssertEqual(state.presentedOrder, ["c", "b", "a"])
        XCTAssertEqual(sends(rollback).count, 1)
        XCTAssertEqual(sends(rollback).first?.threadIds, ["c", "b", "a"])
    }

    func testFullUnpinClearsOutboxAndNeverSendsEmptyPut() throws {
        var state = makeState(["a", "b"], revision: 10)
        let first = try beginDrop(&state, order: ["b", "a"])
        let unpinA = try XCTUnwrap(
            state.beginMembershipChange(threadId: "a", pinned: false).membershipRequest
        )
        let unpinB = try XCTUnwrap(
            state.beginMembershipChange(threadId: "b", pinned: false).membershipRequest
        )
        _ = state.completeReorder(first, page: page(["a", "b"], 11))

        let firstMembership = state.completeMembership(unpinA, page: page(["b"], 12))
        let lastMembership = state.completeMembership(unpinB, page: page([], 13))

        XCTAssertTrue(sends(firstMembership).isEmpty)
        XCTAssertTrue(sends(lastMembership).isEmpty)
        XCTAssertNil(state.outbox)
        XCTAssertEqual(state.presentedOrder, [])
    }

    func testRestartSettlesWhenServerAlreadyEqualsDesiredOrIsEmpty() {
        var equal = GaryxPinnedOrderState(
            gatewayIdentity: gateway,
            restoredOutbox: outbox(["b", "a"], revision: 7)
        )
        let equalUpdate = equal.receivePage(page(["b", "a"], 8), stamp: equal.requestStamp())
        XCTAssertNil(equal.outbox)
        XCTAssertTrue(sends(equalUpdate).isEmpty)

        var empty = GaryxPinnedOrderState(
            gatewayIdentity: gateway,
            restoredOutbox: outbox([], revision: 7)
        )
        let emptyUpdate = empty.receivePage(page(["a"], 8), stamp: empty.requestStamp())
        XCTAssertNil(empty.outbox)
        XCTAssertTrue(sends(emptyUpdate).isEmpty)
        XCTAssertEqual(empty.presentedOrder, ["a"])
        XCTAssertTrue(emptyUpdate.effects.contains(.persist(nil, gatewayIdentity: gateway)))
    }

    func testProjectedEmptyWithLiveUnpinsSurvivesConflictAndRollback() throws {
        var state = makeState(["a", "b"], revision: 10)
        let first = try beginDrop(&state, order: ["b", "a"])
        let unpinA = try XCTUnwrap(
            state.beginMembershipChange(threadId: "a", pinned: false).membershipRequest
        )
        let unpinB = try XCTUnwrap(
            state.beginMembershipChange(threadId: "b", pinned: false).membershipRequest
        )

        let conflict = state.completeReorder(first, page: page(["a", "b"], 11))
        XCTAssertNotNil(state.outbox)
        XCTAssertEqual(state.desiredOrder, [])
        XCTAssertEqual(state.pendingSync, .waitingForMembership)
        XCTAssertTrue(sends(conflict).isEmpty)

        let firstRollback = state.failMembership(unpinA)
        let finalRollback = state.failMembership(unpinB)
        XCTAssertTrue(sends(firstRollback).isEmpty)
        XCTAssertEqual(sends(finalRollback).count, 1)
        let recovery = try XCTUnwrap(sends(finalRollback).first)
        XCTAssertEqual(recovery.threadIds, ["b", "a"])

        _ = state.completeReorder(recovery, page: page(["b", "a"], 12))
        _ = state.receivePage(page(["b", "a"], 12), stamp: state.requestStamp())
        XCTAssertNil(state.outbox)
        XCTAssertEqual(state.presentedOrder, ["b", "a"])
    }

    func testMembershipResponseRaisesFloorBeforeExactlyOneDrain() throws {
        var state = makeState(["a", "b"], revision: 10)
        let first = try beginDrop(&state, order: ["b", "a"])
        let pin = try XCTUnwrap(
            state.beginMembershipChange(threadId: "c", pinned: true).membershipRequest
        )
        _ = state.completeReorder(first, page: page(["a", "b"], 11))

        let completion = state.completeMembership(pin, page: page(["c", "a", "b"], 12))

        XCTAssertEqual(sends(completion).count, 1)
        XCTAssertEqual(sends(completion).first?.expectedRevision, 12)
        XCTAssertEqual(sends(completion).first?.threadIds, ["c", "b", "a"])
    }

    func testMembershipResponseBeforeOldFlightCoalescesThenFollowsUpOnce() throws {
        var state = makeState(["a", "b"], revision: 10)
        let first = try beginDrop(&state, order: ["b", "a"])
        let pin = try XCTUnwrap(
            state.beginMembershipChange(threadId: "c", pinned: true).membershipRequest
        )

        let membership = state.completeMembership(pin, page: page(["c", "a", "b"], 12))
        XCTAssertTrue(sends(membership).isEmpty)
        XCTAssertEqual(state.pendingSync, .coalescedBehindFlight)

        let oldFlight = state.completeReorder(first, page: page(["b", "a"], 11))
        XCTAssertEqual(sends(oldFlight).count, 1)
        XCTAssertEqual(sends(oldFlight).first?.expectedRevision, 12)
        XCTAssertEqual(sends(oldFlight).first?.threadIds, ["c", "b", "a"])
    }

    func testOutboxSettledWhileOldFlightAirborneIsNotRevived() throws {
        var state = makeState(["a", "b"], revision: 10)
        let first = try beginDrop(&state, order: ["b", "a"])
        let pin = try XCTUnwrap(
            state.beginMembershipChange(threadId: "c", pinned: true).membershipRequest
        )

        let membership = state.completeMembership(pin, page: page(["c", "b", "a"], 12))
        XCTAssertNil(state.outbox)
        XCTAssertTrue(sends(membership).isEmpty)

        let late = state.completeReorder(first, page: page(["b", "a"], 11))
        XCTAssertNil(state.outbox)
        XCTAssertTrue(sends(late).isEmpty)
    }

    func testDurableRestartRecoveryAndNewerDropSupersedesOutbox() throws {
        var state = GaryxPinnedOrderState(
            gatewayIdentity: gateway,
            restoredOutbox: outbox(["b", "a"], revision: 10)
        )
        let recovery = state.receivePage(page(["a", "b"], 11), stamp: state.requestStamp())
        let oldFlight = try XCTUnwrap(sends(recovery).first)

        _ = state.beginDrag()
        _ = state.previewDrag(order: ["a", "b"])
        _ = state.acceptDrop()

        XCTAssertEqual(state.outbox?.desiredOrder, ["a", "b"])
        let late = state.completeReorder(oldFlight, page: page(["b", "a"], 12))
        XCTAssertEqual(sends(late).first?.threadIds, ["a", "b"])
    }

    func testRetryableFailureBacksOffAndPermanentFailurePauses() throws {
        var retryable = makeState(["a", "b"], revision: 10)
        let first = try beginDrop(&retryable, order: ["b", "a"])
        _ = retryable.failReorder(first, failure: .retryable(delay: 5), now: 10)
        XCTAssertEqual(retryable.pendingSync, .retryScheduled(attempt: 1, notBefore: 15))
        XCTAssertTrue(sends(retryable.retryTick(now: 14.9)).isEmpty)
        XCTAssertEqual(sends(retryable.retryTick(now: 15)).count, 1)

        var permanent = makeState(["a", "b"], revision: 10)
        let unsupported = try beginDrop(&permanent, order: ["b", "a"])
        _ = permanent.failReorder(unsupported, failure: .permanent(statusCode: 405))
        XCTAssertEqual(permanent.pendingSync, .pausedPermanent(statusCode: 405))
        XCTAssertTrue(permanent.hasPendingSync)
        XCTAssertTrue(sends(permanent.retryTick(now: 100)).isEmpty)
        XCTAssertEqual(sends(permanent.resumePausedSync()).count, 1)
    }

    func testGatewaySwitchClearsDurableOutboxAndActiveFlight() throws {
        var state = makeState(["a", "b"], revision: 10)
        _ = try beginDrop(&state, order: ["b", "a"])

        let update = state.switchGateway(to: "gateway-b")

        XCTAssertNil(state.outbox)
        XCTAssertNil(state.activeReorderFlight)
        XCTAssertEqual(state.highestObservedRevision, 0)
        XCTAssertEqual(state.pendingSync, .settled)
        XCTAssertTrue(update.effects.contains(.persist(nil, gatewayIdentity: gateway)))
    }

    func testSameGatewayReloadRestoresOutboxWithoutStaleTransportTokens() throws {
        var state = makeState(["a", "b"], revision: 10)
        _ = try beginDrop(&state, order: ["b", "a"])
        let persisted = try XCTUnwrap(state.outbox)

        _ = state.reloadCurrentGateway(restoredOutbox: persisted)

        XCTAssertEqual(state.gatewayIdentity, gateway)
        XCTAssertEqual(state.desiredOrder, ["b", "a"])
        XCTAssertEqual(state.pendingSync, .ready)
        XCTAssertNil(state.activeReorderFlight)
    }

    private func makeState(
        _ order: [String],
        revision: Int64
    ) -> GaryxPinnedOrderState {
        GaryxPinnedOrderState(
            gatewayIdentity: gateway,
            initialOrder: order,
            revision: revision
        )
    }

    private func page(_ ids: [String], _ revision: Int64) -> GaryxPinnedOrderPage {
        GaryxPinnedOrderPage(threadIds: ids, revision: revision)
    }

    private func outbox(_ ids: [String], revision: Int64) -> GaryxPinnedOrderOutbox {
        GaryxPinnedOrderOutbox(
            gatewayIdentity: gateway,
            desiredOrder: ids,
            lastKnownRevision: revision
        )
    }

    private func beginDrop(
        _ state: inout GaryxPinnedOrderState,
        order: [String]
    ) throws -> GaryxPinnedOrderReorderRequest {
        _ = state.beginDrag()
        _ = state.previewDrag(order: order)
        return try XCTUnwrap(sends(state.acceptDrop()).first)
    }

    private func sends(_ update: GaryxPinnedOrderUpdate) -> [GaryxPinnedOrderReorderRequest] {
        update.effects.compactMap { effect in
            guard case .sendReorder(let request) = effect else { return nil }
            return request
        }
    }

    private func publications(_ update: GaryxPinnedOrderUpdate) -> [[String]] {
        update.effects.compactMap { effect in
            guard case .publish(let order) = effect else { return nil }
            return order
        }
    }
}
