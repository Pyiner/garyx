import Foundation

extension GaryxMobileModel {
    var favoriteThreads: [GaryxThreadSummary] {
        pendingThreadArchives.visibleThreads(threadFavoritesState.presentedRows)
    }

    var selectedRecentFeedPresentation: GaryxRecentThreadFeedPresentation {
        guard recentThreadFeeds.selectedFilter == .favorites else {
            return recentThreadFeeds.selectedPresentation
        }
        return GaryxRecentThreadFeedPresentation(
            isPrimed: threadFavoritesState.rawRevision != nil,
            isRefreshingHead: threadFavoritesState.activeSnapshotTicket != nil,
            headFailure: threadFavoritesState.snapshotFailed,
            footerState: .hidden
        )
    }

    func threadIsFavorite(_ threadId: String) -> Bool {
        threadFavoritesState.isPresented(threadId: threadId)
    }

    func setThreadFavorite(_ threadId: String, desired: Bool) {
        ensureThreadFavoritesScope()
        runThreadFavoritesEffects(
            applyThreadFavoritesStateTransition { state in
                state.toggle(threadId: threadId, desired: desired)
            }
        )
    }

    func toggleThreadFavorite(_ threadId: String) {
        setThreadFavorite(threadId, desired: !threadIsFavorite(threadId))
    }

    func refreshThreadFavoritesSnapshot() {
        guard hasGatewaySettings else { return }
        ensureThreadFavoritesScope()
        runThreadFavoritesEffects(
            applyThreadFavoritesStateTransition { $0.requestSnapshot() }
        )
    }

    @discardableResult
    func observeThreadStoreIdentity(
        gatewayScope: String,
        runtimeEpoch: UInt64,
        owned: Bool,
        storeIncarnationId: String
    ) -> GaryxStoreIdentityDecision {
        let result = applyThreadFavoritesStateTransition { state in
            state.observeStoreIdentity(
                stamp: GaryxStoreResponseStamp(
                    gatewayScope: gatewayScope,
                    runtimeEpoch: runtimeEpoch,
                    owned: owned
                ),
                responseStoreIncarnationId: storeIncarnationId
            )
        }
        runThreadFavoritesEffects(result.effects)
        return result.decision
    }

    func clearThreadFavoritesRuntime() {
        runThreadFavoritesEffects(
            applyThreadFavoritesStateTransition { state in
                state.replaceGatewayScope("", requestSnapshot: false)
            }
        )
    }

    private func ensureThreadFavoritesScope() {
        let scope = normalizedGatewayURL(gatewayURL)
        guard threadFavoritesState.gatewayScope != scope else { return }
        runThreadFavoritesEffects(
            applyThreadFavoritesStateTransition { state in
                state.replaceGatewayScope(scope, requestSnapshot: false)
            }
        )
    }

    private func runThreadFavoritesEffects(_ effects: [GaryxFavoritesEffect]) {
        for effect in effects {
            switch effect {
            case .surfaceError(_, let message):
                lastError = message
            case .backoff(let stamp, let delayNanoseconds):
                Task { [weak self] in
                    try? await Task.sleep(nanoseconds: delayNanoseconds)
                    guard !Task.isCancelled, let self else { return }
                    self.runThreadFavoritesEffects(
                        self.applyThreadFavoritesStateTransition { state in
                            state.fireBackoff(stamp)
                        }
                    )
                }
            case .snapshot(let ticket):
                Task { [weak self] in
                    guard let self else { return }
                    do {
                        let snapshot = try await self.client().threadFavoritesSnapshot()
                        let effects = self.applyThreadFavoritesStateTransition { state in
                            state.completeSnapshot(
                                ticket: ticket,
                                snapshot: GaryxFavoriteSnapshot(snapshot)
                            )
                        }
                        if self.threadFavoritesState.rawRevision == snapshot.revision,
                           self.threadFavoritesState.storeIncarnationId
                            == snapshot.storeIncarnationId {
                            let visibleSnapshotRows = self.pendingThreadArchives.visibleThreads(
                                snapshot.recent.threads
                            )
                            self.threads = Self.mergedThreadSummaries(
                                self.pendingThreadArchives.visibleThreads(self.threads)
                                    + visibleSnapshotRows
                            )
                        }
                        self.runThreadFavoritesEffects(effects)
                    } catch {
                        self.runThreadFavoritesEffects(
                            self.applyThreadFavoritesStateTransition { state in
                                state.failSnapshot(ticket: ticket)
                            }
                        )
                    }
                }
            case .mutate(let ticket):
                Task { [weak self] in
                    guard let self else { return }
                    let result: GaryxGatewayMutationResult<GaryxThreadFavoritesPage>
                    do {
                        result = try await self.client().setThreadFavorite(
                            threadId: ticket.threadId,
                            favorited: ticket.target,
                            expectedRevision: ticket.expectedRevision,
                            expectedStoreIncarnation: ticket.expectedStoreIncarnation
                        )
                    } catch {
                        result = .notSent(error.localizedDescription)
                    }
                    let settlement: GaryxFavoriteMutationSettlement
                    switch result {
                    case .ok(let page):
                        settlement = .ok(GaryxFavoritePage(page))
                    case .definitiveEndpointResponse(let response):
                        settlement = .definitive(
                            status: response.status,
                            code: response.error.code,
                            message: response.error.message,
                            page: response.decoded.map(GaryxFavoritePage.init)
                        )
                    case .ambiguous(let response):
                        settlement = .ambiguous(message: response.message)
                    case .notSent(let message):
                        settlement = .notSent(message: message)
                    }
                    let effects = self.applyThreadFavoritesStateTransition { state in
                        state.settle(
                            ticket: ticket,
                            settlement: settlement
                        )
                    }
                    self.runThreadFavoritesEffects(effects)
                }
            }
        }
    }

    /// Every favorites reducer transition shares one epoch boundary. A domain
    /// clear from a snapshot, mutation settlement, or Recent identity response
    /// must invalidate both paginated feed lanes before any follow-up effect is
    /// dispatched; otherwise an old incarnation can retain a live pager lane.
    private func applyThreadFavoritesStateTransition<Result>(
        _ transition: (inout GaryxFavoritesState) -> Result
    ) -> Result {
        let previousEpoch = threadFavoritesState.runtimeEpoch
        let result = transition(&threadFavoritesState)
        if threadFavoritesState.runtimeEpoch != previousEpoch {
            recentThreadFeeds.resetFeedData()
        }
        return result
    }
}
