import XCTest
@testable import GaryxMobileCore

final class GaryxCapsulePreviewLoadPlannerTests: XCTestCase {
    // MARK: - Gallery planner (visibility FIFO)

    func testEmptyPlannerHasNoActiveIds() {
        let planner = GaryxCapsulePreviewLoadPlanner(maxActive: 2)
        XCTAssertEqual(planner.activeIds, [])
        XCTAssertFalse(planner.isActive("a"))
    }

    func testAdmitsFirstNVisibleInAppearanceOrder() {
        var planner = GaryxCapsulePreviewLoadPlanner(maxActive: 2)
        XCTAssertTrue(planner.markVisible("a"))
        XCTAssertTrue(planner.markVisible("b"))
        XCTAssertTrue(planner.markVisible("c"))
        XCTAssertEqual(planner.activeIds, ["a", "b"])
        XCTAssertTrue(planner.isActive("a"))
        XCTAssertTrue(planner.isActive("b"))
        XCTAssertFalse(planner.isActive("c"))
    }

    func testHidingAnActiveFreesSlotForNextVisible() {
        var planner = GaryxCapsulePreviewLoadPlanner(maxActive: 2)
        ["a", "b", "c"].forEach { planner.markVisible($0) }
        XCTAssertTrue(planner.markHidden("a"))
        XCTAssertEqual(planner.activeIds, ["b", "c"])
        XCTAssertFalse(planner.isActive("a"))
        XCTAssertTrue(planner.isActive("c"))
    }

    func testIPadCapAdmitsAll() {
        var planner = GaryxCapsulePreviewLoadPlanner(maxActive: 4)
        ["a", "b", "c"].forEach { planner.markVisible($0) }
        XCTAssertEqual(planner.activeIds, ["a", "b", "c"])
    }

    func testMarkVisibleIsIdempotentAndDoesNotReorder() {
        var planner = GaryxCapsulePreviewLoadPlanner(maxActive: 1)
        XCTAssertTrue(planner.markVisible("a"))
        XCTAssertTrue(planner.markVisible("b"))
        XCTAssertFalse(planner.markVisible("a"), "re-marking a visible id is a no-op")
        XCTAssertEqual(planner.visibleIds, ["a", "b"], "order is preserved")
        XCTAssertEqual(planner.activeIds, ["a"])
    }

    func testSetMaxActiveRecomputesAdmission() {
        var planner = GaryxCapsulePreviewLoadPlanner(maxActive: 1)
        ["a", "b", "c"].forEach { planner.markVisible($0) }
        XCTAssertEqual(planner.activeIds, ["a"])
        planner.setMaxActive(3)
        XCTAssertEqual(planner.activeIds, ["a", "b", "c"])
        planner.setMaxActive(0)
        XCTAssertEqual(planner.activeIds, [])
    }

    func testPruneDropsInvalidVisibleIdsKeepingOrder() {
        var planner = GaryxCapsulePreviewLoadPlanner(maxActive: 2)
        ["a", "b", "c"].forEach { planner.markVisible($0) }
        planner.prune(keeping: ["a", "c"])
        XCTAssertEqual(planner.visibleIds, ["a", "c"])
        XCTAssertEqual(planner.activeIds, ["a", "c"])
    }

    // MARK: - Chat-card admission (conversation-level, most-recent N)

    func testChatAdmissionEmpty() {
        XCTAssertEqual(GaryxCapsuleChatCardAdmission.activeKeys(orderedKeys: [], maxActive: 2), [])
    }

    func testChatAdmissionKeepsMostRecentN() {
        // Transcript order is oldest-first; the newest cards (tail) are admitted
        // because the transcript opens scrolled to the bottom.
        let keys = ["t1:a", "t2:b", "t3:c"]
        XCTAssertEqual(
            GaryxCapsuleChatCardAdmission.activeKeys(orderedKeys: keys, maxActive: 2),
            ["t2:b", "t3:c"]
        )
    }

    func testChatAdmissionAdmitsAllWhenCapExceedsCount() {
        let keys = ["t1:a", "t2:b"]
        XCTAssertEqual(GaryxCapsuleChatCardAdmission.activeKeys(orderedKeys: keys, maxActive: 4), keys)
    }

    func testChatAdmissionZeroCapAdmitsNone() {
        XCTAssertEqual(
            GaryxCapsuleChatCardAdmission.activeKeys(orderedKeys: ["t1:a"], maxActive: 0),
            []
        )
    }

    // MARK: - Chat-card presentation

    func testChatCardSubtitle() {
        XCTAssertEqual(GaryxCapsuleChatCardPresentation.subtitle(action: .created), "Created")
        XCTAssertEqual(GaryxCapsuleChatCardPresentation.subtitle(action: .updated), "Updated")
    }
}
