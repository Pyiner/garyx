import Foundation

extension GaryxMobileModel {
    var favoriteThreads: [GaryxThreadSummary] {
        threadFavoritesState.presentedRows
    }

    func threadIsFavorite(_ threadId: String) -> Bool {
        threadFavoritesState.isPresented(threadId: threadId)
    }

    func setThreadFavorite(_ threadId: String, desired: Bool) {
        ensureThreadFavoritesScope()
        runThreadFavoritesEffects(
            threadFavoritesState.toggle(threadId: threadId, desired: desired)
        )
    }

    func toggleThreadFavorite(_ threadId: String) {
        setThreadFavorite(threadId, desired: !threadIsFavorite(threadId))
    }

    func refreshThreadFavoritesSnapshot() {
        guard hasGatewaySettings else { return }
        ensureThreadFavoritesScope()
        runThreadFavoritesEffects(threadFavoritesState.requestSnapshot())
    }

    @discardableResult
    func observeThreadStoreIdentity(
        gatewayScope: String,
        runtimeEpoch: UInt64,
        owned: Bool,
        storeIncarnationId: String
    ) -> GaryxStoreIdentityDecision {
        let result = threadFavoritesState.observeStoreIdentity(
            stamp: GaryxStoreResponseStamp(
                gatewayScope: gatewayScope,
                runtimeEpoch: runtimeEpoch,
                owned: owned
            ),
            responseStoreIncarnationId: storeIncarnationId
        )
        if result.decision == .scopeClear {
            // The favorites runtime epoch invalidates every old response; the
            // feed pager reset makes the same cut structural on All/Chats.
            recentThreadFeeds.resetFeedData()
        }
        runThreadFavoritesEffects(result.effects)
        return result.decision
    }

    func clearThreadFavoritesRuntime() {
        runThreadFavoritesEffects(
            threadFavoritesState.replaceGatewayScope("", requestSnapshot: false)
        )
    }

    private func ensureThreadFavoritesScope() {
        let scope = normalizedGatewayURL(gatewayURL)
        guard threadFavoritesState.gatewayScope != scope else { return }
        runThreadFavoritesEffects(
            threadFavoritesState.replaceGatewayScope(scope, requestSnapshot: false)
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
                        self.threadFavoritesState.fireBackoff(stamp)
                    )
                }
            case .snapshot(let ticket):
                Task { [weak self] in
                    guard let self else { return }
                    do {
                        let snapshot = try await self.client().threadFavoritesSnapshot()
                        let effects = self.threadFavoritesState.completeSnapshot(
                            ticket: ticket,
                            snapshot: GaryxFavoriteSnapshot(snapshot)
                        )
                        if self.threadFavoritesState.rawRevision == snapshot.revision,
                           self.threadFavoritesState.storeIncarnationId
                            == snapshot.storeIncarnationId {
                            self.threads = Self.mergedThreadSummaries(
                                self.threads + snapshot.recent.threads
                            )
                        }
                        self.runThreadFavoritesEffects(effects)
                    } catch {
                        self.runThreadFavoritesEffects(
                            self.threadFavoritesState.failSnapshot(ticket: ticket)
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
                    self.runThreadFavoritesEffects(
                        self.threadFavoritesState.settle(
                            ticket: ticket,
                            settlement: settlement
                        )
                    )
                }
            }
        }
    }
}
