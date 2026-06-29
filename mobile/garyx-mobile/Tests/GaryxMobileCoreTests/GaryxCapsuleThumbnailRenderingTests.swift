import XCTest
@testable import GaryxMobileCore

final class GaryxCapsuleThumbnailRenderingTests: XCTestCase {
    // MARK: - Cache key: hit / invalidation / rendition distinction

    func testSameIdRevisionRenditionHits() {
        let a = GaryxCapsuleThumbnailCacheKey(id: "cap", revision: 3, rendition: .gallery)
        let b = GaryxCapsuleThumbnailCacheKey(id: "cap", revision: 3, rendition: .gallery)
        XCTAssertEqual(a, b)
        XCTAssertEqual(a.storageToken, b.storageToken)
    }

    func testRevisionBumpInvalidatesKey() {
        let r3 = GaryxCapsuleThumbnailCacheKey(id: "cap", revision: 3, rendition: .gallery)
        let r4 = GaryxCapsuleThumbnailCacheKey(id: "cap", revision: 4, rendition: .gallery)
        XCTAssertNotEqual(r3, r4)
        XCTAssertNotEqual(r3.storageToken, r4.storageToken)
    }

    /// The whole point of keying by rendition: a 16:10 gallery image must never
    /// satisfy a 16:9 chat-card lookup for the same capsule + revision.
    func testGalleryAndChatRenditionsAreDistinctKeys() {
        let gallery = GaryxCapsuleThumbnailCacheKey(id: "cap", revision: 3, rendition: .gallery)
        let chat = GaryxCapsuleThumbnailCacheKey(id: "cap", revision: 3, rendition: .chatCard)
        XCTAssertNotEqual(gallery, chat)
        XCTAssertNotEqual(gallery.storageToken, chat.storageToken)
        XCTAssertEqual(gallery.storageToken, "cap.r3.16x10")
        XCTAssertEqual(chat.storageToken, "cap.r3.16x9")
    }

    func testCacheKeyTrimsId() {
        let key = GaryxCapsuleThumbnailCacheKey(id: "  cap  ", revision: 1, rendition: .gallery)
        XCTAssertEqual(key.id, "cap")
    }

    func testRenditionTokensAreStable() {
        XCTAssertEqual(GaryxCapsuleThumbnailRendition.gallery.token, "16x10")
        XCTAssertEqual(GaryxCapsuleThumbnailRendition.chatCard.token, "16x9")
    }

    // MARK: - Snapshot plan geometry (16:10 vs 16:9, cover sizing)

    func testGalleryPlanIs16by10() {
        let plan = GaryxCapsuleThumbnailSnapshotPlan(rendition: .gallery, layoutWidth: 760, scale: 2)
        XCTAssertEqual(plan.layoutWidth, 760)
        XCTAssertEqual(plan.layoutHeight, 475) // 760 * 10/16
        XCTAssertEqual(plan.pixelWidth, 1520)
        XCTAssertEqual(plan.pixelHeight, 950)
    }

    func testChatCardPlanIs16by9() {
        let plan = GaryxCapsuleThumbnailSnapshotPlan(rendition: .chatCard, layoutWidth: 760, scale: 2)
        XCTAssertEqual(plan.layoutWidth, 760)
        XCTAssertEqual(plan.layoutHeight, 428) // round(760 * 9/16 = 427.5)
        XCTAssertEqual(plan.pixelWidth, 1520)
        XCTAssertEqual(plan.pixelHeight, 856) // 428 * 2
    }

    func testPlanScaleDrivesPixelSizeOnly() {
        let p1 = GaryxCapsuleThumbnailSnapshotPlan(rendition: .gallery, layoutWidth: 760, scale: 1)
        let p3 = GaryxCapsuleThumbnailSnapshotPlan(rendition: .gallery, layoutWidth: 760, scale: 3)
        XCTAssertEqual(p1.layoutWidth, p3.layoutWidth)
        XCTAssertEqual(p1.layoutHeight, p3.layoutHeight)
        XCTAssertEqual(p3.pixelWidth, p1.pixelWidth * 3)
        XCTAssertEqual(p3.pixelHeight, p1.pixelHeight * 3)
    }

    // MARK: - Pruner: revision supersede / deletion / rendition coverage

    private func cap(_ id: String, _ revision: Int) -> GaryxCapsuleSummary {
        GaryxCapsuleSummary(id: id, title: id, revision: revision)
    }

    func testPrunerKeepsValidRevisionsAcrossRenditions() {
        let keys = [
            GaryxCapsuleThumbnailCacheKey(id: "a", revision: 2, rendition: .gallery),
            GaryxCapsuleThumbnailCacheKey(id: "a", revision: 2, rendition: .chatCard),
            GaryxCapsuleThumbnailCacheKey(id: "b", revision: 1, rendition: .gallery),
        ]
        let result = GaryxCapsuleThumbnailCachePruner.pruned(keys: keys, validCapsules: [cap("a", 2), cap("b", 1)])
        XCTAssertEqual(Set(result.keep), Set(keys))
        XCTAssertTrue(result.evict.isEmpty)
    }

    func testPrunerEvictsSupersededRevisionEveryRendition() {
        let keys = [
            GaryxCapsuleThumbnailCacheKey(id: "a", revision: 2, rendition: .gallery),
            GaryxCapsuleThumbnailCacheKey(id: "a", revision: 2, rendition: .chatCard),
            GaryxCapsuleThumbnailCacheKey(id: "a", revision: 3, rendition: .gallery),
        ]
        // Authoritative revision is now 3; both rev-2 renditions must be evicted.
        let result = GaryxCapsuleThumbnailCachePruner.pruned(keys: keys, validCapsules: [cap("a", 3)])
        XCTAssertEqual(result.keep, [GaryxCapsuleThumbnailCacheKey(id: "a", revision: 3, rendition: .gallery)])
        XCTAssertEqual(Set(result.evict), Set([
            GaryxCapsuleThumbnailCacheKey(id: "a", revision: 2, rendition: .gallery),
            GaryxCapsuleThumbnailCacheKey(id: "a", revision: 2, rendition: .chatCard),
        ]))
    }

    func testPrunerEvictsDeletedCapsule() {
        let keys = [
            GaryxCapsuleThumbnailCacheKey(id: "a", revision: 1, rendition: .gallery),
            GaryxCapsuleThumbnailCacheKey(id: "b", revision: 1, rendition: .gallery),
        ]
        // `a` no longer in the authoritative list → evicted; `b` kept.
        let result = GaryxCapsuleThumbnailCachePruner.pruned(keys: keys, validCapsules: [cap("b", 1)])
        XCTAssertEqual(result.keep, [GaryxCapsuleThumbnailCacheKey(id: "b", revision: 1, rendition: .gallery)])
        XCTAssertEqual(result.evict, [GaryxCapsuleThumbnailCacheKey(id: "a", revision: 1, rendition: .gallery)])
    }

    func testEvictingCapsuleDropsAllRenditionsAndRevisions() {
        let keys = [
            GaryxCapsuleThumbnailCacheKey(id: "a", revision: 1, rendition: .gallery),
            GaryxCapsuleThumbnailCacheKey(id: "a", revision: 2, rendition: .chatCard),
            GaryxCapsuleThumbnailCacheKey(id: "b", revision: 1, rendition: .gallery),
        ]
        let result = GaryxCapsuleThumbnailCachePruner.evictingCapsule(keys: keys, capsuleId: "a")
        XCTAssertEqual(result.keep, [GaryxCapsuleThumbnailCacheKey(id: "b", revision: 1, rendition: .gallery)])
        XCTAssertEqual(result.evict.count, 2)
        XCTAssertTrue(result.evict.allSatisfy { $0.id == "a" })
    }
}
