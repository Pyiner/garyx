import Foundation

extension GaryxMobileModel {
    func reloadPinnedOrderDomainForCurrentGateway() {
        pinnedOrderReorderTask?.cancel()
        pinnedOrderReorderTask = nil
        pinnedOrderReorderTaskToken = nil

        let identity = currentGatewayScopeId
        let restored = pinnedOrderOutboxStore.loadPinnedOrderOutbox(
            gatewayIdentity: identity
        )
        let update = homeThreadListStore.updatePinnedOrderState { state in
            if state.gatewayIdentity == identity {
                return state.reloadCurrentGateway(restoredOutbox: restored)
            }
            return state.switchGateway(to: identity, restoredOutbox: restored)
        }
        applyPinnedOrderUpdate(update, label: "pins-domain")
        publishPinnedOrder(stateOrder: homeThreadListStore.pinnedOrderState.presentedOrder)
    }

    func capturePinnedOrderRequestStamp() -> GaryxPinnedOrderRequestStamp {
        homeThreadListStore.pinnedOrderState.requestStamp()
    }

    func beginPinnedOrderDrag() {
        let update = homeThreadListStore.updatePinnedOrderState { state in
            state.beginDrag()
        }
        applyPinnedOrderUpdate(update, label: "pins-drag-begin")
    }

    func previewPinnedOrderDrag(_ order: [String]) {
        let update = homeThreadListStore.updatePinnedOrderState { state in
            state.previewDrag(order: order)
        }
        applyPinnedOrderUpdate(update, label: "pins-drag-preview")
    }

    func acceptPinnedOrderDrop() {
        let update = homeThreadListStore.updatePinnedOrderState { state in
            state.acceptDrop(now: pinnedOrderNow)
        }
        applyPinnedOrderUpdate(update, label: "pins-drop")
    }

    func cancelPinnedOrderDrag() {
        let update = homeThreadListStore.updatePinnedOrderState { state in
            state.cancelDrag()
        }
        applyPinnedOrderUpdate(update, label: "pins-drag-cancel")
    }

    @discardableResult
    func beginPinnedOrderMembershipChange(
        threadId: String,
        pinned: Bool
    ) -> GaryxPinnedOrderMembershipRequest? {
        let update = homeThreadListStore.updatePinnedOrderState { state in
            state.beginMembershipChange(
                threadId: threadId,
                pinned: pinned,
                now: pinnedOrderNow
            )
        }
        applyPinnedOrderUpdate(update, label: "pins-membership-begin")
        return update.membershipRequest
    }

    func completePinnedOrderMembershipChange(
        _ request: GaryxPinnedOrderMembershipRequest,
        page: GaryxThreadPinsPage
    ) {
        let update = homeThreadListStore.updatePinnedOrderState { state in
            state.completeMembership(
                request,
                page: GaryxPinnedOrderPage(
                    threadIds: page.threadIds,
                    revision: page.revision
                ),
                now: pinnedOrderNow
            )
        }
        applyPinnedOrderUpdate(update, label: "pins-membership-complete")
    }

    func failPinnedOrderMembershipChange(
        _ request: GaryxPinnedOrderMembershipRequest
    ) {
        let update = homeThreadListStore.updatePinnedOrderState { state in
            state.failMembership(request, now: pinnedOrderNow)
        }
        applyPinnedOrderUpdate(update, label: "pins-membership-fail")
    }

    func applyPinnedThreadIds(
        _ ids: [String],
        revision: Int64 = 0,
        stamp: GaryxPinnedOrderRequestStamp? = nil
    ) {
        let requestStamp = stamp ?? capturePinnedOrderRequestStamp()
        let update = homeThreadListStore.updatePinnedOrderState { state in
            state.receivePage(
                GaryxPinnedOrderPage(threadIds: ids, revision: revision),
                stamp: requestStamp,
                now: pinnedOrderNow
            )
        }
        applyPinnedOrderUpdate(update, label: "pins-page")
    }

    func servicePinnedOrderRetry(source: GaryxThreadListRefreshSource) {
        let update: GaryxPinnedOrderUpdate
        switch homeThreadListStore.pinnedOrderState.pendingSync {
        case .pausedPermanent where source != .backgroundLoop:
            update = homeThreadListStore.updatePinnedOrderState { state in
                state.resumePausedSync(now: pinnedOrderNow)
            }
        case .retryScheduled:
            update = homeThreadListStore.updatePinnedOrderState { state in
                state.retryTick(now: pinnedOrderNow)
            }
        default:
            return
        }
        applyPinnedOrderUpdate(update, label: "pins-retry")
    }

    private var pinnedOrderNow: TimeInterval {
        Date.timeIntervalSinceReferenceDate
    }

    private func applyPinnedOrderUpdate(
        _ update: GaryxPinnedOrderUpdate,
        label: String
    ) {
        var requests: [GaryxPinnedOrderReorderRequest] = []
        var published = false
        let transactionId = homeProjectionGateway.beginTransaction(label: label)
        for effect in update.effects {
            switch effect {
            case .publish(let order):
                published = publishPinnedOrder(stateOrder: order) || published
            case .persist(let outbox, let gatewayIdentity):
                pinnedOrderOutboxStore.savePinnedOrderOutbox(
                    outbox,
                    gatewayIdentity: gatewayIdentity
                )
            case .sendReorder(let request):
                requests.append(request)
            case .noteLocalMutation:
                recentThreadFeeds.noteLocalMutation()
            }
        }
        homeProjectionGateway.endTransaction(transactionId)
        if published {
            persistRecentThreadsWidgetSnapshot()
        }
        for request in requests {
            dispatchPinnedOrderReorder(request)
        }
    }

    @discardableResult
    private func publishPinnedOrder(stateOrder: [String]) -> Bool {
        let visible = pendingThreadArchives.visibleThreadIds(stateOrder)
        let normalized = Self.normalizedPinnedThreadIds(visible)
        guard pinnedThreadIds != normalized else { return false }
        pinnedThreadIds = normalized
        return true
    }

    private func dispatchPinnedOrderReorder(_ request: GaryxPinnedOrderReorderRequest) {
        guard request.stamp.gatewayIdentity == homeThreadListStore.pinnedOrderState.gatewayIdentity,
              homeThreadListStore.pinnedOrderState.activeReorderFlight?.token == request.token,
              pinnedOrderReorderTaskToken == nil else {
            return
        }
        let runtimeGeneration = gatewayRuntimeGeneration
        pinnedOrderReorderTaskToken = request.token
        pinnedOrderReorderTask = Task { [weak self] in
            guard let self else { return }
            do {
                let result = try await client().reorderThreadPins(
                    threadIds: request.threadIds,
                    expectedRevision: request.expectedRevision
                )
                guard runtimeGeneration == gatewayRuntimeGeneration else { return }
                completePinnedOrderReorder(request, page: result.page)
            } catch {
                guard runtimeGeneration == gatewayRuntimeGeneration else { return }
                failPinnedOrderReorder(request, error: error)
            }
        }
    }

    private func completePinnedOrderReorder(
        _ request: GaryxPinnedOrderReorderRequest,
        page: GaryxThreadPinsPage
    ) {
        clearPinnedOrderTaskIfMatching(request.token)
        let update = homeThreadListStore.updatePinnedOrderState { state in
            state.completeReorder(
                request,
                page: GaryxPinnedOrderPage(
                    threadIds: page.threadIds,
                    revision: page.revision
                ),
                now: pinnedOrderNow
            )
        }
        applyPinnedOrderUpdate(update, label: "pins-reorder-complete")
    }

    private func failPinnedOrderReorder(
        _ request: GaryxPinnedOrderReorderRequest,
        error: Error
    ) {
        clearPinnedOrderTaskIfMatching(request.token)
        let failure = pinnedOrderFailure(for: error)
        let update = homeThreadListStore.updatePinnedOrderState { state in
            state.failReorder(
                request,
                failure: failure,
                now: pinnedOrderNow
            )
        }
        applyPinnedOrderUpdate(update, label: "pins-reorder-fail")
    }

    private func clearPinnedOrderTaskIfMatching(_ token: UInt64) {
        guard pinnedOrderReorderTaskToken == token else { return }
        pinnedOrderReorderTask = nil
        pinnedOrderReorderTaskToken = nil
    }

    private func pinnedOrderFailure(for error: Error) -> GaryxPinnedOrderReorderFailure {
        if GaryxGatewayRetryClassifier.isCancellation(error) {
            return .cancelled
        }
        let attempt = homeThreadListStore.pinnedOrderState.nextRetryAttempt
        let policyDelay = GaryxGatewayRetryPolicy.default.delay(forAttempt: attempt)
        if case GaryxGatewayError.httpStatus(let status, _, let retryAfter) = error {
            if GaryxGatewayRetryClassifier.isRetryableStatus(status, idempotent: true) {
                return .retryable(delay: max(policyDelay, retryAfter ?? 0))
            }
            return .permanent(statusCode: status)
        }
        if GaryxGatewayRetryClassifier.isConnectionEstablishmentError(error)
            || GaryxGatewayRetryClassifier.isAmbiguousNetworkError(error) {
            return .retryable(delay: policyDelay)
        }
        return .permanent(statusCode: nil)
    }
}
