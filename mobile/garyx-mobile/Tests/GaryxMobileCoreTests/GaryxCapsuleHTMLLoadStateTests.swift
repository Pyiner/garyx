import XCTest
@testable import GaryxMobileCore

final class GaryxCapsuleHTMLLoadStateTests: XCTestCase {
    func testHTMLCacheKeyUsesIdRevisionAndHash() {
        let v1 = GaryxCapsuleHTMLCacheKey(
            capsule: capsule(id: "capsule-1", revision: 1, htmlSha256: "aaa")
        )
        let v2 = GaryxCapsuleHTMLCacheKey(
            capsule: capsule(id: "capsule-1", revision: 2, htmlSha256: "aaa")
        )
        let v3 = GaryxCapsuleHTMLCacheKey(
            capsule: capsule(id: "capsule-1", revision: 1, htmlSha256: "bbb")
        )

        XCTAssertNotEqual(v1, v2)
        XCTAssertNotEqual(v1, v3)
        XCTAssertEqual(v1.id, "capsule-1")
        XCTAssertEqual(v1.revision, 1)
        XCTAssertEqual(v1.htmlSha256, "aaa")
    }

    func testBeginLoadAndApplyHTMLForCurrentRequest() {
        var state = GaryxCapsuleHTMLLoadState()
        let summary = capsule(id: "capsule-1", revision: 1, htmlSha256: "aaa")
        let key = state.beginHTMLLoad(for: summary)

        XCTAssertEqual(state.selectedCapsuleId, "capsule-1")
        XCTAssertEqual(state.requestedKey, key)
        XCTAssertTrue(state.isLoading)
        XCTAssertNil(state.html)

        XCTAssertTrue(state.applyHTML("<html></html>", for: key))
        XCTAssertFalse(state.isLoading)
        XCTAssertNil(state.requestedKey)
        XCTAssertEqual(state.loadedKey, key)
        XCTAssertEqual(state.html, "<html></html>")
        XCTAssertNil(state.errorMessage)
    }

    func testStaleCompletionIsIgnoredAfterNewSelection() {
        var state = GaryxCapsuleHTMLLoadState()
        let first = capsule(id: "capsule-1", revision: 1, htmlSha256: "aaa")
        let second = capsule(id: "capsule-2", revision: 1, htmlSha256: "bbb")
        let firstKey = state.beginHTMLLoad(for: first)
        let secondKey = state.beginHTMLLoad(for: second)

        XCTAssertFalse(state.applyHTML("stale", for: firstKey))
        XCTAssertTrue(state.isLoading)
        XCTAssertEqual(state.requestedKey, secondKey)
        XCTAssertNil(state.html)
    }

    func testApplyCachedHTMLInstallsLoadedStateWithoutLoading() {
        var state = GaryxCapsuleHTMLLoadState()
        let summary = capsule(id: "capsule-1", revision: 1, htmlSha256: "aaa")
        state.select(summary)
        let key = GaryxCapsuleHTMLCacheKey(capsule: summary)

        XCTAssertTrue(state.applyCachedHTML("cached", for: key))
        XCTAssertFalse(state.isLoading)
        XCTAssertEqual(state.loadedKey, key)
        XCTAssertEqual(state.html, "cached")
    }

    func testFailureClearsLoadingAndKeepsOnlyCurrentError() {
        var state = GaryxCapsuleHTMLLoadState()
        let summary = capsule(id: "capsule-1", revision: 1, htmlSha256: "aaa")
        let key = state.beginHTMLLoad(for: summary)

        XCTAssertTrue(state.applyHTMLFailure("network failed", for: key))
        XCTAssertFalse(state.isLoading)
        XCTAssertNil(state.requestedKey)
        XCTAssertNil(state.loadedKey)
        XCTAssertNil(state.html)
        XCTAssertEqual(state.errorMessage, "network failed")
    }

    func testRemoveClearsSelectedCapsuleState() {
        var state = GaryxCapsuleHTMLLoadState()
        let summary = capsule(id: "capsule-1", revision: 1, htmlSha256: "aaa")
        let key = state.beginHTMLLoad(for: summary)
        XCTAssertTrue(state.applyHTML("html", for: key))

        state.remove(id: "capsule-1")

        XCTAssertNil(state.selectedCapsuleId)
        XCTAssertNil(state.loadedKey)
        XCTAssertNil(state.html)
        XCTAssertFalse(state.isLoading)
    }

    private func capsule(
        id: String,
        revision: Int,
        htmlSha256: String
    ) -> GaryxCapsuleSummary {
        GaryxCapsuleSummary(
            id: id,
            title: "Capsule",
            htmlSha256: htmlSha256,
            byteSize: 10,
            revision: revision
        )
    }
}
