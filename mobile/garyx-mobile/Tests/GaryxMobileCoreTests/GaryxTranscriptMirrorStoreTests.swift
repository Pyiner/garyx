import XCTest
@testable import GaryxMobileCore

/// TASK-1751 P1 — the mirror store guarantees every mutation bumps the
/// generation, closing the design-review v3 "clearTranscriptCache bypasses the
/// generation gate" gap structurally (there is no way to mutate the mirror
/// without bumping).
final class GaryxTranscriptMirrorStoreTests: XCTestCase {
    private func window(_ threadId: String, _ text: String) -> GaryxCachedTranscript {
        GaryxCachedTranscript(
            threadId: threadId,
            savedAt: Date(timeIntervalSince1970: 0),
            messages: [GaryxTranscriptMessage(index: 0, role: .assistant, text: text)],
            hasMoreBefore: false,
            nextBeforeIndex: nil
        )
    }

    func testGenerationStartsAtZero() {
        let store = GaryxTranscriptMirrorStore()
        XCTAssertEqual(store.generation(for: "t"), 0)
        XCTAssertNil(store.snapshot(for: "t"))
        XCTAssertFalse(store.contains("t"))
    }

    func testSetBumpsGeneration() {
        var store = GaryxTranscriptMirrorStore()
        store.set(window("t", "a"), for: "t")
        XCTAssertEqual(store.generation(for: "t"), 1)
        XCTAssertEqual(store.snapshot(for: "t")?.messages.first?.text, "a")
        store.set(window("t", "b"), for: "t")
        XCTAssertEqual(store.generation(for: "t"), 2)
        XCTAssertEqual(store.snapshot(for: "t")?.messages.first?.text, "b")
    }

    /// The finding-3 core: clearing (set nil) must also bump, so an in-flight
    /// restore that captured the pre-clear generation aborts.
    func testClearBumpsGeneration() {
        var store = GaryxTranscriptMirrorStore()
        store.set(window("t", "a"), for: "t")
        let afterSet = store.generation(for: "t")
        store.set(nil, for: "t")
        XCTAssertNil(store.snapshot(for: "t"))
        XCTAssertFalse(store.contains("t"))
        XCTAssertGreaterThan(store.generation(for: "t"), afterSet, "clear must bump the generation")
    }

    func testGenerationsArePerThread() {
        var store = GaryxTranscriptMirrorStore()
        store.set(window("a", "x"), for: "a")
        store.set(window("a", "y"), for: "a")
        store.set(window("b", "z"), for: "b")
        XCTAssertEqual(store.generation(for: "a"), 2)
        XCTAssertEqual(store.generation(for: "b"), 1)
    }

    func testClearAllBumpsEveryPresentThreadAndKeepsCountersMonotonic() {
        var store = GaryxTranscriptMirrorStore()
        store.set(window("a", "x"), for: "a")
        store.set(window("b", "y"), for: "b")
        let genABefore = store.generation(for: "a")
        let genBBefore = store.generation(for: "b")
        store.clearAll()
        XCTAssertNil(store.snapshot(for: "a"))
        XCTAssertNil(store.snapshot(for: "b"))
        XCTAssertGreaterThan(store.generation(for: "a"), genABefore, "clearAll bumps present threads")
        XCTAssertGreaterThan(store.generation(for: "b"), genBBefore)
        // Re-setting the same id after clearAll continues from the bumped value,
        // never resetting to a low number that could masquerade as unchanged.
        store.set(window("a", "z"), for: "a")
        XCTAssertGreaterThan(store.generation(for: "a"), genABefore + 1)
    }

    /// End-to-end with the restore policy: a clear between restore spawn and
    /// apply flips shouldApply/shouldSeedMirror to false via the bumped
    /// generation — the "clear after control rewrite beats restore" invariant.
    func testClearBetweenSpawnAndApplyAbortsRestore() {
        var store = GaryxTranscriptMirrorStore()
        store.set(window("t", "old disk window"), for: "t")
        let capturedMirrorGen = store.generation(for: "t") // restore spawns here

        // A stream control-rewrite recovery clears the cache mid-restore.
        store.set(nil, for: "t")

        let state = GaryxColdOpenRestorePolicy.State(
            restoredThreadId: "t",
            selectedThreadId: "t",
            capturedGeneration: 1,
            currentGeneration: 1,
            capturedMirrorGeneration: capturedMirrorGen,
            currentMirrorGeneration: store.generation(for: "t"),
            threadHistoryLoaded: false,
            hasRenderSnapshot: false,
            hasMessages: false
        )
        XCTAssertFalse(GaryxColdOpenRestorePolicy.shouldApply(state),
                       "restore must not apply after a clear bumped the mirror generation")
        XCTAssertFalse(GaryxColdOpenRestorePolicy.shouldSeedMirror(state),
                       "restore must not seed after a clear bumped the mirror generation")
    }
}
