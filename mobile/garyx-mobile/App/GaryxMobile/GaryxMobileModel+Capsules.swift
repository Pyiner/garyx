import Foundation
import SwiftUI
import UIKit

/// Outcome of resolving a capsule's rendered thumbnail image by
/// `(id, revision, rendition)`. `.deleted`/`.failed` mirror the HTML loader's
/// deletion-vs-transient distinction.
enum GaryxCapsuleThumbnailResult: Equatable {
    case image(UIImage)
    case deleted
    case failed
}

/// Outcome of fetching a capsule's preview HTML by id. `/serve` is the single
/// deletion authority: a 404 means the capsule was deleted (render-state markers
/// outlive the row), while transient/5xx/offline failures stay retryable and are
/// never mislabeled as deleted.
enum GaryxCapsulePreviewHTMLResult: Equatable {
    case html(String)
    case deleted
    case failed
}

extension GaryxMobileModel {
    /// Cheap check of whether the selected thread's render state carries any
    /// capsule cards, read straight off the raw snapshot rows so it can drive
    /// route-time deletion validation without the cost of the full
    /// `selectedThreadTurnRows()` mapping on every render.
    var selectedThreadHasCapsuleCards: Bool {
        guard let threadId = selectedThread?.id,
              let snapshot = renderSnapshot(for: threadId) else { return false }
        return snapshot.rows.contains { row in
            if case let .userTurn(turn) = row {
                return !turn.capsuleCards.isEmpty
            }
            return false
        }
    }

    /// Single entry point for every Capsule catalog read. Concurrent callers
    /// share one GET, while a trigger arriving during that GET advances
    /// `requestedTicket`; the worker must then issue one immediate trailing GET
    /// before it can finish. Only this worker commits catalog state.
    @discardableResult
    func refreshCapsules(
        reportFailure: Bool = true
    ) async -> Result<[GaryxCapsuleSummary], Error> {
        guard hasGatewaySettings else {
            let result: Result<[GaryxCapsuleSummary], Error> = .failure(
                GaryxGatewayError.invalidURL(gatewayURL)
            )
            if reportFailure, case let .failure(error) = result {
                lastError = displayMessage(for: error)
            }
            return result
        }

        capsuleCatalogRequestedTicket &+= 1
        let callerTicket = capsuleCatalogRequestedTicket
        var latestResult: Result<[GaryxCapsuleSummary], Error> = .success(capsules)

        // A worker can finish its final check just before another caller marks a
        // trailing ticket. Looping through the caller's own ticket closes that
        // narrow completion race without ever allowing two workers.
        while capsuleCatalogFinishedTicket < callerTicket {
            let (token, task) = capsuleCatalogWorker()
            latestResult = await task.value
            if capsuleCatalogRefreshTaskToken == token {
                capsuleCatalogRefreshTask = nil
                capsuleCatalogRefreshTaskToken = nil
            }
        }

        if reportFailure, case let .failure(error) = latestResult,
           !GaryxGatewayRetryClassifier.isCancellation(error) {
            lastError = displayMessage(for: error)
        }
        return latestResult
    }

    private func capsuleCatalogWorker(
    ) -> (UUID, Task<Result<[GaryxCapsuleSummary], Error>, Never>) {
        if let task = capsuleCatalogRefreshTask,
           let token = capsuleCatalogRefreshTaskToken {
            return (token, task)
        }
        let token = UUID()
        let task = Task { @MainActor [weak self] () -> Result<[GaryxCapsuleSummary], Error> in
            guard let self else { return .failure(CancellationError()) }
            return await self.runCapsuleCatalogWorker()
        }
        capsuleCatalogRefreshTaskToken = token
        capsuleCatalogRefreshTask = task
        return (token, task)
    }

    private func runCapsuleCatalogWorker() async -> Result<[GaryxCapsuleSummary], Error> {
        var latestResult: Result<[GaryxCapsuleSummary], Error> = .success(capsules)
        while true {
            let attemptTicket = capsuleCatalogRequestedTicket
            let runtimeGeneration = gatewayRequestToken
            let favoritesGeneration = capsuleFavoriteState.favoritesGeneration
            do {
                let nextCapsules = try await client().listCapsules()
                guard runtimeGeneration == gatewayRequestToken else {
                    latestResult = .failure(CancellationError())
                    capsuleCatalogFinishedTicket = max(capsuleCatalogFinishedTicket, attemptTicket)
                    return latestResult
                }
                // The coordinator is the unique writer. The ticket guard remains
                // as a defensive latest-wins barrier against any future bypass.
                if attemptTicket >= capsuleCatalogCommittedTicket {
                    mergeCapsulesFromRefresh(
                        nextCapsules,
                        capturedFavoritesGeneration: favoritesGeneration
                    )
                    capsuleCatalogCommittedTicket = attemptTicket
                    persistCatalogCacheSnapshot()
                }
                latestResult = .success(nextCapsules)
            } catch {
                latestResult = .failure(error)
            }
            capsuleCatalogFinishedTicket = max(capsuleCatalogFinishedTicket, attemptTicket)
            guard capsuleCatalogRequestedTicket > attemptTicket else { return latestResult }
            // A trigger arrived while the request was in flight: immediately
            // consume the newest ticket in a second, trailing GET.
        }
    }

    func filteredCapsules(for tab: GaryxCapsuleGalleryTab) -> [GaryxCapsuleSummary] {
        tab.filter(capsules, favoriteState: capsuleFavoriteState)
    }

    func isCapsuleFavorited(_ capsule: GaryxCapsuleSummary) -> Bool {
        GaryxCapsuleFavoriteReducer.isFavorited(capsule, state: capsuleFavoriteState)
    }

    /// Resolve the source-thread metadata used by the Capsule action panel.
    /// Prefer the existing thread-list projection so its title/avatar stay in
    /// lockstep with the list; only issue a focused read when pagination has not
    /// brought that source thread into memory.
    func capsuleSourceThreadSummary(threadId: String) async -> GaryxThreadSummary? {
        let id = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !id.isEmpty else { return nil }
        if let cached = sidebarThreadSummary(for: id) { return cached }
        guard hasGatewaySettings else { return nil }
        let runtimeGeneration = gatewayRequestToken
        do {
            let thread = try await client().getThread(threadId: id)
            guard runtimeGeneration == gatewayRequestToken else { return nil }
            return thread
        } catch {
            return nil
        }
    }

    func toggleCapsuleFavorite(_ capsule: GaryxCapsuleSummary) async {
        guard hasGatewaySettings else { return }
        let transition = GaryxCapsuleFavoriteReducer.toggle(
            capsules: capsules,
            state: capsuleFavoriteState,
            capsuleId: capsule.id,
            favorited: !isCapsuleFavorited(capsule)
        )
        applyCapsuleFavoriteTransition(transition)
        guard var effect = transition.effect else { return }
        GaryxMobileHaptics.shared.play(.capsuleFavoriteChanged)

        let runtimeGeneration = gatewayRequestToken
        while true {
            do {
                let response = try await client().setCapsuleFavorite(
                    id: effect.capsuleId,
                    favorited: effect.favorited
                )
                guard runtimeGeneration == gatewayRequestToken else { return }
                let settled = GaryxCapsuleFavoriteReducer.succeeded(
                    capsules: capsules,
                    state: capsuleFavoriteState,
                    capsuleId: effect.capsuleId,
                    response: response
                )
                applyCapsuleFavoriteTransition(settled)
                persistCatalogCacheSnapshot()
                guard let followUp = settled.effect else { return }
                effect = followUp
            } catch {
                guard runtimeGeneration == gatewayRequestToken else { return }
                let failed = GaryxCapsuleFavoriteReducer.failed(
                    capsules: capsules,
                    state: capsuleFavoriteState,
                    capsuleId: effect.capsuleId
                )
                applyCapsuleFavoriteTransition(failed)
                lastError = displayMessage(for: error)
                return
            }
        }
    }

    func mergeCapsulesFromRefresh(
        _ refreshedCapsules: [GaryxCapsuleSummary],
        capturedFavoritesGeneration: Int
    ) {
        applyCapsuleFavoriteTransition(
            GaryxCapsuleFavoriteReducer.mergingRefresh(
                currentCapsules: capsules,
                refreshedCapsules: refreshedCapsules,
                state: capsuleFavoriteState,
                capturedGeneration: capturedFavoritesGeneration
            )
        )
    }

    private func applyCapsuleFavoriteTransition(_ transition: GaryxCapsuleFavoriteTransition) {
        capsuleFavoriteState = transition.state
        // The existing didSet prune path is revision-keyed. Favorite-only
        // changes therefore keep both HTML and thumbnail cache entries alive.
        capsules = transition.capsules
    }

    /// Shared preview-HTML loader for gallery thumbnails, chat-card thumbnails,
    /// and the focused preview. Loads by id directly (not via a `capsules`
    /// lookup) so a deleted or synthetic capsule still reaches `/serve` and
    /// reports `.deleted`. Cache-first unless `forceRefresh`; the focused preview
    /// force-refreshes so the full-screen surface is never stale.
    func loadCapsulePreviewHTML(
        capsuleId: String,
        revision: Int,
        forceRefresh: Bool = false
    ) async -> GaryxCapsulePreviewHTMLResult {
        guard hasGatewaySettings else { return .failed }
        let key = GaryxCapsuleHTMLCacheKey(id: capsuleId, revision: revision)
        if !forceRefresh, let cached = capsuleHTMLCache[key] {
            return .html(cached)
        }
        let runtimeGeneration = gatewayRequestToken
        do {
            let html = try await client().capsuleHTML(id: capsuleId)
            guard runtimeGeneration == gatewayRequestToken else { return .failed }
            capsuleHTMLCache[key] = html
            return .html(html)
        } catch let error as GaryxGatewayError {
            guard runtimeGeneration == gatewayRequestToken else { return .failed }
            if case .httpStatus(404, _, _) = error {
                // The whole capsule is gone. Centralized eviction so *every*
                // surface re-validates: HTML cache, rendered-thumbnail memory +
                // disk (all renditions/revisions), and the cache epoch — even
                // when the focused preview discovered the 404 and only the
                // thumbnail caches hold this id.
                await evictDeletedCapsuleCaches(capsuleId: capsuleId)
                return .deleted
            }
            return .failed
        } catch {
            return .failed
        }
    }

    func beginFocusedCapsuleHTMLRequest(for key: GaryxCapsulePreviewLoadKey) -> UUID {
        focusedCapsuleHTMLRequestGate.begin(key)
    }

    func invalidateFocusedCapsuleHTMLRequests() {
        focusedCapsuleHTMLRequestGate.invalidate()
    }

    func acceptsFocusedCapsuleHTMLRequest(
        key: GaryxCapsulePreviewLoadKey,
        token: UUID
    ) -> Bool {
        focusedCapsuleHTMLRequestGate.accepts(key: key, token: token)
    }

    /// One focused-preview network attempt. Inner Gateway retries are disabled;
    /// `GaryxCapsuleFocusedPreviewLoader` is the sole bounded retry owner. A
    /// stale key/token returns `.stale` and can never mutate cache or UI state.
    func loadFocusedCapsulePreviewHTMLAttempt(
        key: GaryxCapsulePreviewLoadKey,
        token: UUID
    ) async throws -> GaryxFocusedCapsuleHTMLAttempt {
        try Task.checkCancellation()
        guard hasGatewaySettings else { throw GaryxGatewayError.invalidURL(gatewayURL) }
        let runtimeGeneration = gatewayRequestToken
        do {
            let html = try await client().capsuleHTML(id: key.id, allowsRetry: false)
            try Task.checkCancellation()
            guard runtimeGeneration == gatewayRequestToken else { throw CancellationError() }
            guard acceptsFocusedCapsuleHTMLRequest(key: key, token: token) else { return .stale }
            if let revision = key.projectedRevision {
                capsuleHTMLCache[GaryxCapsuleHTMLCacheKey(id: key.id, revision: revision)] = html
            }
            return .html(html)
        } catch {
            if GaryxGatewayRetryClassifier.isCancellation(error) {
                throw CancellationError()
            }
            guard runtimeGeneration == gatewayRequestToken else { throw CancellationError() }
            guard acceptsFocusedCapsuleHTMLRequest(key: key, token: token) else { return .stale }
            if case GaryxGatewayError.httpStatus(404, _, _) = error {
                await evictDeletedCapsuleCaches(capsuleId: key.id)
                return .deleted
            }
            throw error
        }
    }

    /// Centralized `/serve` 404 handling: the capsule is gone, so evict its HTML
    /// cache **and** its rendered-thumbnail memory + disk caches (every
    /// rendition/revision), then bump the cache epoch so mounted gallery/chat
    /// thumbnails for the same id re-validate to `.deleted` instead of serving a
    /// stale cached PNG. Bumps even when no HTML entry was present (the focused
    /// preview can discover the 404 while only the thumbnail caches hold the id).
    func evictDeletedCapsuleCaches(capsuleId: String) async {
        let htmlEvicted = GaryxCapsuleHTMLCachePruner.evictingCapsule(
            cache: capsuleHTMLCache,
            capsuleId: capsuleId
        )
        capsuleHTMLCache = htmlEvicted.cache
        let memoryEvicted = capsuleThumbnailMemory.evict(capsuleId: capsuleId)
        let diskEvicted = await capsuleThumbnailStore.evict(capsuleId: capsuleId)
        if htmlEvicted.didEvict || memoryEvicted || diskEvicted {
            capsuleHTMLCacheEpoch &+= 1
        }
    }

    /// Present a focused capsule preview above the current conversation (chat
    /// capsule-card tap). Resolves the full summary from the loaded catalog when
    /// available, else synthesizes a minimal summary so a since-deleted capsule
    /// still presents and resolves to "Capsule deleted" via `/serve` 404 — never
    /// switching to the Capsules panel or showing a route-not-found alert.
    func presentConversationCapsulePreview(_ capsuleId: String) async {
        let id = capsuleId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !id.isEmpty else { return }
        if !capsules.contains(where: { $0.id == id }) {
            await refreshCapsules()
        }
        let fallback = capsules.first { $0.id == id }
            ?? GaryxCapsuleSummary(id: id, title: "Capsule")
        conversationCapsulePreview = GaryxCapsulePreviewSelection(capsule: fallback)
    }

    func deleteCapsule(_ capsule: GaryxCapsuleSummary) async {
        let runtimeGeneration = gatewayRequestToken
        do {
            _ = try await client().deleteCapsule(id: capsule.id)
            guard runtimeGeneration == gatewayRequestToken else { return }
            if galleryFocusedCapsule?.id == capsule.id { galleryFocusedCapsule = nil }
            if conversationCapsulePreview?.id == capsule.id { conversationCapsulePreview = nil }
            capsuleFavoriteState.mutations.removeValue(forKey: capsule.id)
            // didSet prunes the deleted capsule's preview HTML and bumps the epoch.
            capsules.removeAll { $0.id == capsule.id }
            persistCatalogCacheSnapshot()
        } catch {
            guard runtimeGeneration == gatewayRequestToken else { return }
            lastError = displayMessage(for: error)
        }
    }

    /// Prune cached preview HTML to the authoritative capsules list and bump the
    /// cache epoch when anything was evicted. Internal (not private) so the
    /// `capsules` didSet in the main model file can call it.
    func pruneCapsuleHTMLCache(validCapsules: [GaryxCapsuleSummary]) {
        let result = GaryxCapsuleHTMLCachePruner.pruned(
            cache: capsuleHTMLCache,
            validCapsules: validCapsules
        )
        capsuleHTMLCache = result.cache
        // Rendered thumbnails follow the same authority: drop deleted capsules
        // from the in-memory image cache now (synchronous, so a remotely-deleted
        // capsule's chat-card thumbnail re-validates) and prune the disk cache
        // off the main actor. Bump the epoch when anything is evicted so mounted
        // thumbnails re-reconcile.
        let validIds = Set(validCapsules.map { $0.id.trimmingCharacters(in: .whitespacesAndNewlines) })
        let memoryEvicted = capsuleThumbnailMemory.retainOnly(validIds: validIds)
        if result.didEvict || memoryEvicted {
            capsuleHTMLCacheEpoch &+= 1
        }
        Task { [weak self] in
            guard let self else { return }
            let diskEvicted = await self.capsuleThumbnailStore.pruneToValid(validCapsules)
            if diskEvicted { self.capsuleHTMLCacheEpoch &+= 1 }
        }
    }

    /// Resolve a capsule's rendered thumbnail image for a surface (gallery 16:10,
    /// chat card 16:9). Memory → disk → render-once. The gallery and chat cards
    /// display this image with **no live `WKWebView`**; the one-shot render on a
    /// miss is concurrency-capped by `GaryxCapsuleThumbnailRenderer`, so visible
    /// cards are never starved (A1) and the crop is a fixed 16:rendition cover
    /// over an opaque backing (A2).
    func capsuleThumbnail(
        capsuleId: String,
        revision: Int,
        rendition: GaryxCapsuleThumbnailRendition
    ) async -> GaryxCapsuleThumbnailResult {
        let key = GaryxCapsuleThumbnailCacheKey(id: capsuleId, revision: revision, rendition: rendition)
        if let cached = capsuleThumbnailMemory.image(for: key) {
            return .image(cached)
        }
        if let data = await capsuleThumbnailStore.data(for: key),
           let image = await Self.decodeThumbnail(data) {
            capsuleThumbnailMemory.set(image, for: key)
            return .image(image)
        }
        // Miss: reuse the HTML loader (it owns `/serve`, the 404 deletion
        // authority, and transient/offline handling), then render once. A 404
        // inside `loadCapsulePreviewHTML` already evicted every cache for this id
        // via `evictDeletedCapsuleCaches`, so `.deleted` just reports the state.
        switch await loadCapsulePreviewHTML(capsuleId: capsuleId, revision: revision) {
        case .deleted:
            return .deleted
        case .failed:
            return .failed
        case let .html(html):
            let plan = GaryxCapsuleThumbnailSnapshotPlan(rendition: rendition)
            guard let png = await capsuleThumbnailRenderer.renderPNG(html: html, plan: plan),
                  let image = await Self.decodeThumbnail(png) else {
                return .failed
            }
            await capsuleThumbnailStore.store(png, for: key)
            capsuleThumbnailMemory.set(image, for: key)
            return .image(image)
        }
    }

    private static func decodeThumbnail(_ data: Data) async -> UIImage? {
        await Task.detached(priority: .utility) { UIImage(data: data) }.value
    }
}
