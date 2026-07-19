import Foundation

extension GaryxMobileModel {
    var favoriteThreads: [GaryxThreadSummary] {
        pendingThreadArchives.visibleThreads(
            threadSummaryCache.summaries(for: threadFavoritesProvider.snapshot.orderedThreadIds)
        )
    }

    var selectedRecentFeedPresentation: GaryxRecentThreadFeedPresentation {
        guard recentThreadFeeds.selectedFilter == .favorites else {
            return recentThreadFeeds.selectedPresentation
        }
        return GaryxRecentThreadFeedPresentation(
            isPrimed: threadFavoritesProvider.snapshot.isPrimed,
            isRefreshingHead: threadFavoritesProvider.snapshot.isRefreshing,
            headFailure: threadFavoritesProvider.snapshot.headFailure,
            footerState: .hidden
        )
    }

    func threadIsFavorite(_ threadId: String) -> Bool {
        threadFavoritesState.isPresented(threadId: threadId)
    }

    func setThreadFavorite(_ threadId: String, desired: Bool) {
        ensureThreadFavoritesScope()
        let previous = threadIsFavorite(threadId)
        runThreadFavoritesEffects(
            threadFavoritesProvider.toggle(threadId: threadId, desired: desired)
        )
        publishThreadSummaryState()
        if threadIsFavorite(threadId) != previous {
            GaryxMobileHaptics.shared.play(.threadFavoriteChanged)
        }
    }

    func toggleThreadFavorite(_ threadId: String) {
        setThreadFavorite(threadId, desired: !threadIsFavorite(threadId))
    }

    func refreshThreadFavoritesSnapshot() {
        Task { [weak self] in
            await self?.requestThreadFavoritesSnapshot()
        }
    }

    func refreshThreadFavoritesSnapshotAndWait() async {
        await requestThreadFavoritesSnapshot()
        while let task = threadFavoritesSnapshotTask {
            await task.value
        }
    }

    private func requestThreadFavoritesSnapshot() async {
        guard hasGatewaySettings else { return }
        ensureThreadFavoritesScope()
        let resolution = await resolveThreadSummaryCapability()
        guard !Task.isCancelled else { return }
        let transition = threadFavoritesProvider.requestSnapshot(for: resolution)
        runThreadFavoritesEffects(transition.effects)
        if transition.effects.isEmpty, !resolution.becameSupported {
            runThreadFavoritesEffects(threadFavoritesProvider.requestRefresh())
        }
        publishThreadSummaryState()
    }

    @discardableResult
    func observeThreadStoreIdentity(
        gatewayScope: String,
        runtimeEpoch: UInt64,
        owned: Bool,
        storeIncarnationId: String
    ) -> GaryxStoreIdentityDecision {
        let result = threadFavoritesProvider.observeStoreIdentity(
            stamp: GaryxStoreResponseStamp(
                gatewayScope: gatewayScope,
                runtimeEpoch: runtimeEpoch,
                owned: owned
            ),
            responseStoreIncarnationId: storeIncarnationId
        )
        runThreadFavoritesEffects(result.effects)
        if result.decision == .scopeClear {
            recentThreadFeeds.resetFeedData()
        }
        publishThreadSummaryState()
        return result.decision
    }

    func clearThreadFavoritesRuntime() {
        cancelThreadFavoritesSnapshotTransport()
        runThreadFavoritesEffects(threadFavoritesProvider.replaceGatewayScope(""))
        recentThreadFeeds.resetFeedData()
        publishThreadSummaryState()
    }

    func ensureThreadFavoritesScope() {
        let scope = normalizedGatewayURL(gatewayURL)
        guard threadFavoritesState.gatewayScope != scope else { return }
        cancelThreadFavoritesSnapshotTransport()
        runThreadFavoritesEffects(threadFavoritesProvider.replaceGatewayScope(scope))
        recentThreadFeeds.resetFeedData()
    }

    func runThreadFavoritesEffects(_ effects: [GaryxFavoritesEffect]) {
        for effect in effects {
            switch effect {
            case .surfaceError(_, let message):
                lastError = message
            case .backoff(let stamp, let delayNanoseconds):
                Task { [weak self] in
                    try? await Task.sleep(nanoseconds: delayNanoseconds)
                    guard !Task.isCancelled, let self else { return }
                    runThreadFavoritesEffects(threadFavoritesProvider.fireBackoff(stamp))
                    publishThreadSummaryState()
                }
            case .snapshot(let ticket):
                threadFavoritesSnapshotTask?.cancel()
                let taskToken = UUID()
                threadFavoritesSnapshotTaskToken = taskToken
                threadFavoritesSnapshotTask = Task { [weak self] in
                    guard let self else { return }
                    defer {
                        if threadFavoritesSnapshotTaskToken == taskToken {
                            threadFavoritesSnapshotTask = nil
                            threadFavoritesSnapshotTaskToken = nil
                        }
                    }
                    do {
                        let snapshot = try await client().threadFavoritesSnapshot(
                            includeSummaries: ticket.requestFlavor == .enhanced
                        )
                        var acceptedSnapshot = GaryxFavoriteSnapshot(snapshot)
                        acceptedSnapshot.rows = pendingThreadArchives.visibleThreads(
                            acceptedSnapshot.rows
                        )
                        if let summaries = acceptedSnapshot.summaryLookupRows {
                            acceptedSnapshot.summaryLookupRows = pendingThreadArchives
                                .visibleThreads(summaries)
                        }
                        let previousFavoritesRuntimeEpoch = threadFavoritesState.runtimeEpoch
                        let completion = threadFavoritesProvider.completeSnapshot(
                            ticket: ticket,
                            snapshot: acceptedSnapshot
                        )
                        let identityReset = threadFavoritesState.runtimeEpoch
                            != previousFavoritesRuntimeEpoch
                        if identityReset {
                            recentThreadFeeds.resetFeedData()
                            refreshRecentThreadLeases()
                        }
                        runThreadFavoritesEffects(completion.effects)
                        if completion.accepted {
                            refreshRecentThreadLeases()
                            publishThreadSummaryState()
                            persistRecentThreadsWidgetSnapshot()
                        } else if identityReset {
                            publishThreadSummaryState()
                            persistRecentThreadsWidgetSnapshot()
                        }
                    } catch {
                        runThreadFavoritesEffects(
                            threadFavoritesProvider.failSnapshot(ticket: ticket)
                        )
                        publishThreadSummaryState()
                    }
                }
            case .mutate(let ticket):
                Task { [weak self] in
                    guard let self else { return }
                    let result: GaryxGatewayMutationResult<GaryxThreadFavoritesPage>
                    do {
                        result = try await client().setThreadFavorite(
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
                    runThreadFavoritesEffects(
                        threadFavoritesProvider.settle(ticket: ticket, settlement: settlement)
                    )
                    if threadFavoritesProvider.state.inFlight[ticket.threadId] == nil,
                       threadFavoritesProvider.state.intents[ticket.threadId] == nil {
                        let mutationId = nextThreadMutationId(
                            kind: "favorite",
                            threadId: ticket.threadId
                        )
                        threadMutationHubStore.value.fanOutFavoritesCommitted(
                            mutationId: mutationId,
                            threadId: ticket.threadId,
                            favorited: threadFavoritesState.isPresented(threadId: ticket.threadId)
                        )
                    }
                    publishThreadSummaryState()
                }
            }
        }
    }

    func nextThreadMutationId(kind: String, threadId: String) -> GaryxThreadMutationID {
        defer { nextThreadMutationSequence &+= 1 }
        return GaryxThreadMutationID(
            "app:\(kind):\(threadId):\(nextThreadMutationSequence)"
        )
    }

    func cancelThreadFavoritesSnapshotTransport() {
        threadFavoritesSnapshotTask?.cancel()
        threadFavoritesSnapshotTask = nil
        threadFavoritesSnapshotTaskToken = nil
    }
}
