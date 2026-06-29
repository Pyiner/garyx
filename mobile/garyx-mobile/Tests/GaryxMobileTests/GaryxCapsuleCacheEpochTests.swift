import XCTest
@testable import GaryxMobile

/// The capsule preview-HTML cache must invalidate on every capsules-list update
/// so a remotely-deleted capsule's cached page cannot be served, and must bump
/// the cache epoch when anything is evicted so already-mounted thumbnails
/// re-reconcile. These assert the model wiring around `pruneCapsuleHTMLCache`.
@MainActor
final class GaryxCapsuleCacheEpochTests: XCTestCase {
    func testCapsulesUpdatePrunesDeletedHTMLAndBumpsEpoch() {
        let model = makeModel()
        model.capsuleHTMLCache = [
            GaryxCapsuleHTMLCacheKey(id: "keep", revision: 1): "<keep/>",
            GaryxCapsuleHTMLCacheKey(id: "gone", revision: 1): "<gone/>",
        ]
        let epochBefore = model.capsuleHTMLCacheEpoch

        // A capsules-list update missing "gone" (deleted) prunes its cached HTML.
        model.capsules = [GaryxCapsuleSummary(id: "keep", title: "Keep", revision: 1)]

        XCTAssertEqual(model.capsuleHTMLCacheEpoch, epochBefore + 1)
        XCTAssertNil(model.capsuleHTMLCache[GaryxCapsuleHTMLCacheKey(id: "gone", revision: 1)])
        XCTAssertEqual(model.capsuleHTMLCache[GaryxCapsuleHTMLCacheKey(id: "keep", revision: 1)], "<keep/>")
    }

    func testCapsulesUpdateWithoutEvictionDoesNotBumpEpoch() {
        let model = makeModel()
        model.capsuleHTMLCache = [GaryxCapsuleHTMLCacheKey(id: "keep", revision: 1): "<keep/>"]
        let epochBefore = model.capsuleHTMLCacheEpoch

        // The cached capsule still exists; adding another evicts nothing.
        model.capsules = [
            GaryxCapsuleSummary(id: "keep", title: "Keep", revision: 1),
            GaryxCapsuleSummary(id: "new", title: "New", revision: 1),
        ]

        XCTAssertEqual(model.capsuleHTMLCacheEpoch, epochBefore)
        XCTAssertEqual(model.capsuleHTMLCache[GaryxCapsuleHTMLCacheKey(id: "keep", revision: 1)], "<keep/>")
    }

    func testSupersededRevisionIsEvictedAndBumpsEpoch() {
        let model = makeModel()
        model.capsuleHTMLCache = [GaryxCapsuleHTMLCacheKey(id: "doc", revision: 1): "<v1/>"]
        let epochBefore = model.capsuleHTMLCacheEpoch

        // An update bumps the capsule's revision; the old (id, revision) entry
        // is no longer valid and is evicted.
        model.capsules = [GaryxCapsuleSummary(id: "doc", title: "Doc", revision: 2)]

        XCTAssertEqual(model.capsuleHTMLCacheEpoch, epochBefore + 1)
        XCTAssertNil(model.capsuleHTMLCache[GaryxCapsuleHTMLCacheKey(id: "doc", revision: 1)])
    }

    private func makeModel() -> GaryxMobileModel {
        let suiteName = "GaryxCapsuleCacheEpochTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defaults.removePersistentDomain(forName: suiteName)
        return GaryxMobileModel(defaults: defaults)
    }
}
