import XCTest
@testable import GaryxMobileCore

/// #TASK-1458 — the thumbnail must fill the frame horizontally by *scaling the
/// page* (never by painting a backing behind centered content). These cover the
/// pure scale/translate math; the in-page measurement + the end-to-end pixel
/// result are covered by the Chromium reproduction harness (cross-engine).
final class GaryxCapsuleThumbnailFillTests: XCTestCase {
    private let eps = 0.001

    /// Content that already spans the full width (full-bleed, or an author
    /// `max-width` ≥ device width that caps to the viewport) needs no transform.
    func testContentThatAlreadyFillsNeedsNoTransform() {
        XCTAssertNil(GaryxCapsuleThumbnailFill.fillTransform(
            contentLeft: 0, contentWidth: 390, viewportWidth: 390
        ))
    }

    /// Sub-pixel slack (rounding from the layout engine) is treated as "fills".
    func testSubPixelGutterIsTreatedAsFilled() {
        XCTAssertNil(GaryxCapsuleThumbnailFill.fillTransform(
            contentLeft: 0.4, contentWidth: 389.3, viewportWidth: 390
        ))
    }

    /// The narrow case (`max-width:300` → 364px border-box centered in a 390
    /// device-width viewport, 13px gutters): scale up to fill, shift flush-left.
    func testNarrowCenteredContentScalesToFillFlushLeft() {
        let t = GaryxCapsuleThumbnailFill.fillTransform(
            contentLeft: 13, contentWidth: 364, viewportWidth: 390
        )
        let unwrapped = try? XCTUnwrap(t)
        XCTAssertNotNil(unwrapped)
        XCTAssertEqual(t!.scale, 390.0 / 364.0, accuracy: eps)        // ≈ 1.0714
        XCTAssertEqual(t!.translateX, -13.0 * (390.0 / 364.0), accuracy: eps) // ≈ -13.93
        // Verify it actually maps the content edges onto [0, 390].
        let mappedLeft = 13.0 * t!.scale + t!.translateX
        let mappedRight = (13.0 + 364.0) * t!.scale + t!.translateX
        XCTAssertEqual(mappedLeft, 0, accuracy: eps)
        XCTAssertEqual(mappedRight, 390, accuracy: eps)
    }

    /// A left gutter with no scale-up still warrants a transform (shift only is
    /// impossible with a single uniform scale, so any left offset triggers fill).
    func testLeftGutterTriggersTransformEvenIfWidthClose() {
        let t = GaryxCapsuleThumbnailFill.fillTransform(
            contentLeft: 8, contentWidth: 382, viewportWidth: 390
        )
        XCTAssertNotNil(t)
    }

    /// Robustness: invalid measurements never produce a transform.
    func testGuardsAgainstZeroInputs() {
        XCTAssertNil(GaryxCapsuleThumbnailFill.fillTransform(contentLeft: 0, contentWidth: 0, viewportWidth: 390))
        XCTAssertNil(GaryxCapsuleThumbnailFill.fillTransform(contentLeft: 0, contentWidth: 390, viewportWidth: 0))
    }

    /// The injected JS must carry the same arithmetic and target so it cannot
    /// silently diverge from `fillTransform`.
    func testFillScriptMirrorsTheTransformArithmetic() {
        let js = GaryxCapsuleThumbnailFill.fillScript
        XCTAssertTrue(js.contains("window.innerWidth"))
        XCTAssertTrue(js.contains("getBoundingClientRect"))
        XCTAssertTrue(js.contains("vw / width"))
        XCTAssertTrue(js.contains("scale <= 1.005 && left <= 1"))
        XCTAssertTrue(js.contains("translateX(' + (-left * scale)"))
        XCTAssertTrue(js.contains("transformOrigin = 'top left'"))
    }
}
