import WebKit
import XCTest
@testable import GaryxMobile
// Core types (GaryxCapsuleViewport, …SnapshotPlan) are compiled into the
// GaryxMobile app module in the xcodeproj build, so no separate Core import.

/// #TASK-1478 — a capsule thumbnail is a frozen capture; it must contain no
/// scrollbar. A capsule is authored full-screen so its content is far taller
/// than the short captured band, which can make the engine paint a root/inner
/// `overflow` scrollbar into the snapshot. These drive the real shipped
/// `GaryxCapsuleThumbnailRenderer` on the iOS simulator (real WKWebView +
/// `takeSnapshot`) and assert the captured PNG's right edge carries no scrollbar.
///
/// Each page is a solid-color full-bleed block taller than the band: the fill
/// transform is a no-op (content already spans the width), so the whole captured
/// frame is that one color — any pixel at the right edge that differs from it is
/// scrollbar chrome.
@MainActor
final class GaryxCapsuleThumbnailScrollbarTests: XCTestCase {
    // Distinctive solid fill so a gray/white/translucent scrollbar stands out.
    private let fill = (r: 0x33, g: 0x66, b: 0xff)

    /// Full-bleed block far taller than the captured band → root scrollbar.
    private let rootOverflowHTML = """
    <!doctype html><html><head><meta charset="utf-8"><style>
    html,body{margin:0;padding:0}
    .bleed{background:#3366ff;width:100%;height:2200px}
    </style></head><body><div class="bleed"></div></body></html>
    """

    /// A full-bleed `overflow:auto` block with content far taller than itself →
    /// inner-container scrollbar at the right edge.
    private let innerOverflowHTML = """
    <!doctype html><html><head><meta charset="utf-8"><style>
    html,body{margin:0;padding:0}
    .panel{background:#3366ff;width:100%;height:2200px;overflow:auto}
    .panel .inner{height:9000px}
    </style></head><body><div class="panel"><div class="inner"></div></div></body></html>
    """

    private var hostWindow: UIWindow?

    override func setUp() async throws {
        try await super.setUp()
        // A live key window so layout and paint actually run — snapshots of an
        // unhosted web view come back blank.
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 430, height: 932))
        window.isHidden = false
        window.makeKeyAndVisible()
        hostWindow = window
    }

    override func tearDown() async throws {
        hostWindow?.isHidden = true
        hostWindow = nil
        try await super.tearDown()
    }

    func testRootOverflowThumbnailHasNoRightEdgeScrollbar() async throws {
        try await assertNoScrollbar(html: rootOverflowHTML, name: "root-overflow")
    }

    func testInnerOverflowThumbnailHasNoRightEdgeScrollbar() async throws {
        try await assertNoScrollbar(html: innerOverflowHTML, name: "inner-overflow")
    }

    // MARK: helpers

    private func assertNoScrollbar(html: String, name: String) async throws {
        let renderer = GaryxCapsuleThumbnailRenderer(maxConcurrent: 1)
        let plan = GaryxCapsuleThumbnailSnapshotPlan(rendition: .gallery)
        guard let data = await renderer.renderPNG(html: html, plan: plan),
              let image = UIImage(data: data) else {
            return XCTFail("[\(name)] renderer returned no PNG")
        }
        // Write the PNG so it can be eyeballed (path printed in the test log).
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("task-1478-\(name).png")
        try? data.write(to: url)
        print("[#TASK-1478] wrote \(name) thumbnail → \(url.path)")

        let result = measure(image)
        XCTAssertTrue(result.rendered, "[\(name)] snapshot did not paint the fill color — inconclusive")
        XCTAssertLessThan(
            result.edgeMismatch, 0.01,
            "[\(name)] thumbnail has a scrollbar at the right edge (mismatch \(result.edgeMismatch))"
        )
    }

    /// Sample the center as the rendered fill color (sanity that paint happened),
    /// then the fraction of right-edge pixels that differ from it (scrollbar
    /// chrome). Scanning only the rightmost ~5 CSS px keeps content out of frame.
    private func measure(_ image: UIImage) -> (rendered: Bool, edgeMismatch: Double) {
        guard let cg = image.cgImage else { return (false, 1) }
        let width = cg.width
        let height = cg.height
        guard width > 0, height > 0 else { return (false, 1) }
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        guard let ctx = CGContext(
            data: &pixels,
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: width * 4,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else { return (false, 1) }
        ctx.draw(cg, in: CGRect(x: 0, y: 0, width: width, height: height))

        func px(_ x: Int, _ y: Int) -> (Int, Int, Int) {
            let i = (y * width + x) * 4
            return (Int(pixels[i]), Int(pixels[i + 1]), Int(pixels[i + 2]))
        }
        let center = px(width / 2, height / 2)
        let rendered = abs(center.0 - fill.r) + abs(center.1 - fill.g) + abs(center.2 - fill.b) < 40

        // Rightmost ~5 CSS px (image is 3x device width → ~15px).
        let edge = max(1, Int((Double(width) / 390.0 * 5.0).rounded()))
        var mismatch = 0
        var total = 0
        for y in 0..<height {
            for x in (width - edge)..<width {
                let p = px(x, y)
                total += 1
                if abs(p.0 - center.0) + abs(p.1 - center.1) + abs(p.2 - center.2) > 30 {
                    mismatch += 1
                }
            }
        }
        return (rendered, total > 0 ? Double(mismatch) / Double(total) : 0)
    }
}
