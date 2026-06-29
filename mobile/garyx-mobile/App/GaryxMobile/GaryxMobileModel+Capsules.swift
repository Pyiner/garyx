import Foundation
import SwiftUI

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
                // Evict any stale entry and bump the epoch so sibling thumbnails
                // of the same capsule re-validate to `.deleted` too.
                if capsuleHTMLCache[key] != nil {
                    capsuleHTMLCache[key] = nil
                    capsuleHTMLCacheEpoch &+= 1
                }
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
        if result.didEvict {
            capsuleHTMLCacheEpoch &+= 1
        }
    }
}
