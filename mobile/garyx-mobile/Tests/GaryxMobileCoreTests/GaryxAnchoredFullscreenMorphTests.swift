import XCTest
@testable import GaryxMobileCore

final class GaryxAnchoredFullscreenMorphTests: XCTestCase {
    private let source = CGRect(x: 18, y: 132, width: 170, height: 106.25)
    private let container = CGSize(width: 390, height: 844)

    func testEndpointsAreThumbnailAndFullCanvas() {
        let collapsed = layout(0)
        XCTAssertEqual(collapsed.frame, source)
        XCTAssertEqual(collapsed.cornerRadius, 12)
        XCTAssertEqual(collapsed.contentOpacity, 0)
        XCTAssertEqual(collapsed.scrimOpacity, 0)

        let expanded = layout(1)
        XCTAssertEqual(expanded.frame, CGRect(origin: .zero, size: container))
        XCTAssertEqual(expanded.cornerRadius, 0)
        XCTAssertEqual(expanded.contentOpacity, 1)
        XCTAssertEqual(expanded.scrimOpacity, 0.12)
    }

    func testOpeningAndClosingSampleTheExactSamePathInReverse() {
        let progressSamples = (0...10).map { CGFloat($0) / 10 }
        let opening = progressSamples.map(layout)
        let closing = progressSamples.reversed().map(layout)

        XCTAssertEqual(opening.count, closing.count)
        for (outbound, inbound) in zip(opening, closing.reversed()) {
            XCTAssertEqual(outbound.frame.origin.x, inbound.frame.origin.x, accuracy: 0.000_1)
            XCTAssertEqual(outbound.frame.origin.y, inbound.frame.origin.y, accuracy: 0.000_1)
            XCTAssertEqual(outbound.frame.width, inbound.frame.width, accuracy: 0.000_1)
            XCTAssertEqual(outbound.frame.height, inbound.frame.height, accuracy: 0.000_1)
            XCTAssertEqual(outbound.cornerRadius, inbound.cornerRadius, accuracy: 0.000_1)
            XCTAssertEqual(outbound.contentOpacity, inbound.contentOpacity, accuracy: 0.000_1)
        }
    }

    func testHalfwayFrameInterpolatesEveryEdge() {
        let halfway = layout(0.5)
        XCTAssertEqual(halfway.frame.minX, 9)
        XCTAssertEqual(halfway.frame.minY, 66)
        XCTAssertEqual(halfway.frame.width, 280)
        XCTAssertEqual(halfway.frame.height, 475.125)
        XCTAssertEqual(halfway.cornerRadius, 6)
        XCTAssertEqual(halfway.contentOpacity, 0.5)
        XCTAssertEqual(halfway.scrimOpacity, 0.06)
    }

    func testInvalidOrOffscreenAnchorUsesStableFallback() {
        let expected = GaryxAnchoredFullscreenMorphGeometry.fallbackSourceRect(
            containerSize: container
        )
        for invalid in [
            CGRect.zero,
            CGRect(x: 500, y: 900, width: 100, height: 100),
        ] {
            XCTAssertEqual(
                GaryxAnchoredFullscreenMorphGeometry.layout(
                    progress: 0,
                    sourceRect: invalid,
                    containerSize: container
                ).frame,
                expected
            )
        }
    }

    private func layout(_ progress: CGFloat) -> GaryxAnchoredFullscreenMorphLayout {
        GaryxAnchoredFullscreenMorphGeometry.layout(
            progress: progress,
            sourceRect: source,
            containerSize: container
        )
    }
}
