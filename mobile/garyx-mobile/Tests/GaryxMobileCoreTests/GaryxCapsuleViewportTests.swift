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

    // MARK: Thumbnail scrollbar hiding (#TASK-1478)

    func testScrollbarHidingStyleHidesWebkitAndStandardScrollbars() {
        let style = GaryxCapsuleViewport.scrollbarHidingStyle
        XCTAssertTrue(style.contains("::-webkit-scrollbar"))
        XCTAssertTrue(style.contains("display:none!important"))
        XCTAssertTrue(style.contains("scrollbar-width:none"))
    }

    func testHidingScrollbarsInsertsAfterHeadTag() {
        let html = "<html><head><title>x</title></head><body>b</body></html>"
        let output = GaryxCapsuleViewport.hidingScrollbars(in: html)
        XCTAssertTrue(
            output.contains(#"<head><style id="garyx-thumbnail-scrollbar-hide""#),
            "the style must be inserted immediately after the <head> open tag"
        )
        XCTAssertTrue(output.contains("<title>x</title>"))
    }

    func testHidingScrollbarsPrependsWhenNoHead() {
        let output = GaryxCapsuleViewport.hidingScrollbars(in: "<main>demo</main>")
        XCTAssertTrue(output.hasPrefix(#"<style id="garyx-thumbnail-scrollbar-hide""#))
        XCTAssertTrue(output.hasSuffix("<main>demo</main>"))
    }

    func testHidingScrollbarsIsIdempotent() {
        let once = GaryxCapsuleViewport.hidingScrollbars(in: "<head></head><body>b</body>")
        let twice = GaryxCapsuleViewport.hidingScrollbars(in: once)
        XCTAssertEqual(once, twice, "a second prepare must not inject a duplicate style")
        XCTAssertEqual(once.components(separatedBy: "garyx-thumbnail-scrollbar-hide").count - 1, 1)
    }

    func testPreparingForThumbnailAddsBothViewportAndScrollbarHiding() {
        let output = GaryxCapsuleViewport.preparingForThumbnail(in: servedFragment)
        XCTAssertTrue(output.contains("width=device-width"), "viewport must be injected")
        XCTAssertTrue(output.contains("::-webkit-scrollbar"), "scrollbars must be hidden")
        XCTAssertTrue(output.contains("<header><h1>Capsule</h1></header>"), "markup preserved")
    }

    func testPreparingForThumbnailRespectsAuthorViewportButStillHidesScrollbars() {
        let html = #"<head><meta name="viewport" content="width=320"></head><body>b</body>"#
        let output = GaryxCapsuleViewport.preparingForThumbnail(in: html)
        XCTAssertTrue(output.contains("width=320"), "an author-declared viewport is respected")
        XCTAssertFalse(output.contains("width=device-width"))
        XCTAssertTrue(output.contains("garyx-thumbnail-scrollbar-hide"), "scrollbars still hidden")
    }
}
