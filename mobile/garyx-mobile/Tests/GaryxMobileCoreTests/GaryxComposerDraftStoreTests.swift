import XCTest
@testable import GaryxMobileCore

final class GaryxComposerDraftStoreTests: XCTestCase {
    func testSetAndReadCurrentForActiveKey() {
        var store = GaryxComposerDraftStore(activeKey: "t1")
        XCTAssertEqual(store.current, "")
        store.setCurrent("hello")
        XCTAssertEqual(store.current, "hello")
    }

    func testEmptyTextDropsTheEntry() {
        var store = GaryxComposerDraftStore(activeKey: "t1")
        store.setCurrent("hi")
        store.setCurrent("")
        XCTAssertEqual(store.current, "")
        XCTAssertNil(store.drafts["t1"], "an empty draft is not stored as \"\"")
    }

    func testSwitchingThreadsPreservesEachDraft() {
        // The reported bug: type in A, switch to B, switch back to A — A is intact.
        var store = GaryxComposerDraftStore(activeKey: "A")
        store.setCurrent("draft for A")
        XCTAssertTrue(store.switchTo("B"))
        XCTAssertEqual(store.current, "", "B starts empty")
        store.setCurrent("draft for B")
        XCTAssertTrue(store.switchTo("A"))
        XCTAssertEqual(store.current, "draft for A", "A's draft survived the round trip")
        XCTAssertTrue(store.switchTo("B"))
        XCTAssertEqual(store.current, "draft for B")
    }

    func testSwitchToSameKeyIsNoOp() {
        var store = GaryxComposerDraftStore(activeKey: "A")
        store.setCurrent("x")
        XCTAssertFalse(store.switchTo("A"), "no reload needed when the key is unchanged")
        XCTAssertEqual(store.current, "x")
    }

    func testNewThreadDraftPreservedAcrossThreadVisit() {
        var store = GaryxComposerDraftStore() // active = newThreadKey
        store.setCurrent("a new thread idea")
        XCTAssertTrue(store.switchTo("existing-thread"))
        store.setCurrent("reply in existing")
        XCTAssertTrue(store.switchTo(GaryxComposerDraftStore.newThreadKey))
        XCTAssertEqual(store.current, "a new thread idea")
    }

    func testResetClearsOnlyActiveDraft() {
        var store = GaryxComposerDraftStore(activeKey: "A")
        store.setCurrent("A text")
        store.switchTo("B")
        store.setCurrent("B text")
        store.reset() // active is B
        XCTAssertEqual(store.current, "")
        store.switchTo("A")
        XCTAssertEqual(store.current, "A text", "reset must not touch other threads")
    }

    func testDiscardActiveThreadFallsBackToNewThread() {
        var store = GaryxComposerDraftStore(activeKey: "A")
        store.setCurrent("A text")
        XCTAssertTrue(store.discard(threadId: "A"), "discarding the active thread requires a reload")
        XCTAssertEqual(store.activeKey, GaryxComposerDraftStore.newThreadKey)
        XCTAssertEqual(store.current, "")
    }

    func testDiscardInactiveThreadKeepsActiveContext() {
        var store = GaryxComposerDraftStore(activeKey: "A")
        store.setCurrent("A text")
        store.switchTo("B")
        store.setCurrent("B text")
        XCTAssertFalse(store.discard(threadId: "A"), "discarding a background thread needs no reload")
        XCTAssertEqual(store.activeKey, "B")
        XCTAssertEqual(store.current, "B text")
        store.switchTo("A")
        XCTAssertEqual(store.current, "", "A's draft was discarded")
    }

    func testClearAllResetsEverything() {
        var store = GaryxComposerDraftStore(activeKey: "A")
        store.setCurrent("A text")
        store.switchTo("B")
        store.setCurrent("B text")
        store.clearAll()
        XCTAssertTrue(store.drafts.isEmpty)
        XCTAssertEqual(store.activeKey, GaryxComposerDraftStore.newThreadKey)
        XCTAssertEqual(store.current, "")
    }
}
