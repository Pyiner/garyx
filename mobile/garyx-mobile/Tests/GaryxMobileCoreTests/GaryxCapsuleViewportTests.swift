import XCTest
@testable import GaryxMobileCore

/// #TASK-1453 problem B — the iOS capsule full-screen detail page must fill the
/// device width and disable zoom. Served capsule HTML carries only a CSP meta
/// (no viewport), so the detail web view injects a device-width, non-zoomable
/// viewport.
final class GaryxCapsuleViewportTests: XCTestCase {
    /// The real served shape: CSP meta prepended to a self-contained card
    /// fragment with no `<head>` and no viewport.
    private let servedFragment = """
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'">\
    <header><h1>Capsule</h1></header><main style="max-width:720px;margin:0 auto">body</main>
    """

    func testInjectsViewportWhenAbsent() {
        let output = GaryxCapsuleViewport.ensuringMobileViewport(in: servedFragment)
        XCTAssertTrue(
            output.contains(#"name="viewport""#),
            "the served fragment carries no viewport, so one must be injected"
        )
        XCTAssertTrue(output.contains("width=device-width"))
        XCTAssertTrue(output.contains("user-scalable=no"))
        XCTAssertTrue(output.contains("maximum-scale=1"))
        // The original markup is preserved verbatim.
        XCTAssertTrue(output.contains("<header><h1>Capsule</h1></header>"))
        XCTAssertTrue(output.contains("Content-Security-Policy"))
    }

    func testInsertsAfterHeadTagWhenPresent() {
        let html = "<html><head><title>x</title></head><body>b</body></html>"
        let output = GaryxCapsuleViewport.ensuringMobileViewport(in: html)
        XCTAssertTrue(
            output.contains(#"<head><meta name="viewport""#),
            "viewport must be inserted immediately after the <head> open tag"
        )
        XCTAssertTrue(output.contains("<title>x</title>"))
    }

    func testPrependsWhenNoHead() {
        let output = GaryxCapsuleViewport.ensuringMobileViewport(in: "<main>demo</main>")
        XCTAssertTrue(output.hasPrefix(#"<meta name="viewport""#))
        XCTAssertTrue(output.hasSuffix("<main>demo</main>"))
    }

    func testLeavesExistingViewportUntouched() {
        let html = #"<head><meta name="viewport" content="width=320"></head><body>b</body>"#
        let output = GaryxCapsuleViewport.ensuringMobileViewport(in: html)
        XCTAssertEqual(output, html, "an author-declared viewport is respected")
        // Exactly one viewport meta — no duplicate injected.
        XCTAssertEqual(output.components(separatedBy: "name=\"viewport\"").count - 1, 1)
    }

    func testDetectsViewportCaseInsensitivelyAndSingleQuoted() {
        XCTAssertTrue(GaryxCapsuleViewport.declaresViewport(#"<META NAME='viewport' CONTENT='width=device-width'>"#))
        XCTAssertFalse(GaryxCapsuleViewport.declaresViewport("<meta charset=\"utf-8\">"))
    }
}
