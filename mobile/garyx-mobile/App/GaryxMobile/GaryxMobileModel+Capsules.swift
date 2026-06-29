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

    func refreshCapsules() async {
        guard hasGatewaySettings else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let nextCapsules = try await client().listCapsules()
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            // `capsules` didSet prunes stale preview HTML and bumps the cache
            // epoch, so deleted capsules drop out of the cache on every refresh.
            capsules = nextCapsules
            persistCatalogCacheSnapshot()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
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
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let html = try await client().capsuleHTML(id: capsuleId)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return .failed }
            capsuleHTMLCache[key] = html
            return .html(html)
        } catch let error as GaryxGatewayError {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return .failed }
            if case .httpStatus(404, _) = error {
                // The whole capsule is gone: evict every cached revision (not just
                // this key) and bump the epoch so sibling thumbnails — including a
                // card at another revision — re-validate to `.deleted` too.
                let evicted = GaryxCapsuleHTMLCachePruner.evictingCapsule(
                    cache: capsuleHTMLCache,
                    capsuleId: capsuleId
                )
                capsuleHTMLCache = evicted.cache
                if evicted.didEvict { capsuleHTMLCacheEpoch &+= 1 }
                return .deleted
            }
            return .failed
        } catch {
            return .failed
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
        conversationCapsulePreview = capsules.first { $0.id == id }
            ?? GaryxCapsuleSummary(id: id, title: "Capsule")
    }

    func deleteCapsule(_ capsule: GaryxCapsuleSummary) async {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            _ = try await client().deleteCapsule(id: capsule.id)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            if galleryFocusedCapsule?.id == capsule.id { galleryFocusedCapsule = nil }
            if conversationCapsulePreview?.id == capsule.id { conversationCapsulePreview = nil }
            // didSet prunes the deleted capsule's preview HTML and bumps the epoch.
            capsules.removeAll { $0.id == capsule.id }
            persistCatalogCacheSnapshot()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
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
        // authority, and transient/offline handling), then render once.
        switch await loadCapsulePreviewHTML(capsuleId: capsuleId, revision: revision) {
        case .deleted:
            capsuleThumbnailMemory.evict(capsuleId: capsuleId)
            if await capsuleThumbnailStore.evict(capsuleId: capsuleId) {
                capsuleHTMLCacheEpoch &+= 1
            }
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
