import Foundation
import SwiftUI

extension GaryxMobileModel {
    var selectedCapsule: GaryxCapsuleSummary? {
        guard let id = capsuleHTMLState.selectedCapsuleId else { return nil }
        return capsules.first { $0.id == id }
    }

    var isCapsuleHTMLLoaded: Bool {
        guard let capsule = selectedCapsule else { return false }
        return capsuleHTMLState.loadedKey == GaryxCapsuleHTMLCacheKey(capsule: capsule)
            && capsuleHTMLState.html != nil
    }

    func refreshCapsules() async {
        guard hasGatewaySettings else { return }
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            let nextCapsules = try await client().listCapsules()
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            capsules = nextCapsules
            pruneCapsuleHTMLCache(validCapsules: nextCapsules)
            if let selectedId = capsuleHTMLState.selectedCapsuleId,
               !nextCapsules.contains(where: { $0.id == selectedId }) {
                capsuleHTMLState.remove(id: selectedId)
            }
            persistCatalogCacheSnapshot()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func openCapsule(_ capsule: GaryxCapsuleSummary) async {
        capsuleHTMLState.select(capsule)
        await loadSelectedCapsuleHTML()
    }

    func loadSelectedCapsuleHTML(forceRefresh: Bool = false) async {
        guard hasGatewaySettings else { return }
        guard let capsule = selectedCapsule else { return }
        let key = GaryxCapsuleHTMLCacheKey(capsule: capsule)
        if !forceRefresh, let cached = capsuleHTMLCache[key] {
            _ = capsuleHTMLState.applyCachedHTML(cached, for: key)
            return
        }

        let runtimeGeneration = gatewayRuntimeGeneration
        let requestedKey = capsuleHTMLState.beginHTMLLoad(for: capsule)
        do {
            let html = try await client().capsuleHTML(id: capsule.id)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            capsuleHTMLCache[requestedKey] = html
            _ = capsuleHTMLState.applyHTML(html, for: requestedKey)
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            let message = displayMessage(for: error)
            if capsuleHTMLState.applyHTMLFailure(message, for: requestedKey) {
                lastError = message
            }
        }
    }

    func deleteCapsule(_ capsule: GaryxCapsuleSummary) async {
        let runtimeGeneration = gatewayRuntimeGeneration
        do {
            _ = try await client().deleteCapsule(id: capsule.id)
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            capsules.removeAll { $0.id == capsule.id }
            capsuleHTMLCache = capsuleHTMLCache.filter { $0.key.id != capsule.id }
            capsuleHTMLState.remove(id: capsule.id)
            persistCatalogCacheSnapshot()
        } catch {
            guard runtimeGeneration == gatewayRuntimeGeneration else { return }
            lastError = displayMessage(for: error)
        }
    }

    func clearCapsuleDetailState() {
        capsuleHTMLState = GaryxCapsuleHTMLLoadState()
    }

    private func pruneCapsuleHTMLCache(validCapsules: [GaryxCapsuleSummary]) {
        let validKeys = Set(validCapsules.map(GaryxCapsuleHTMLCacheKey.init))
        capsuleHTMLCache = capsuleHTMLCache.filter { validKeys.contains($0.key) }
    }
}
