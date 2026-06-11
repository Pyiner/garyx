import XCTest
@testable import GaryxMobileCore

final class GaryxFilePathLinkDetectorTests: XCTestCase {
    func testDetectsAbsolutePath() {
        let text = "完整路径: /Users/test/repos/garyx/docs/design/mobile-gateway-switcher-prd.html"
        let links = GaryxFilePathLinkDetector.detect(in: text)
        XCTAssertEqual(
            links.map(\.target),
            ["/Users/test/repos/garyx/docs/design/mobile-gateway-switcher-prd.html"]
        )
        XCTAssertTrue(links[0].isAbsolute)
    }

    func testDetectsRelativePathInsideParentheses() {
        let text = "(docs/design/mobile-gateway-switcher-prd.html)"
        let links = GaryxFilePathLinkDetector.detect(in: text)
        XCTAssertEqual(links.map(\.target), ["docs/design/mobile-gateway-switcher-prd.html"])
        XCTAssertFalse(links[0].isAbsolute)
    }

    func testCharacterOffsetsMatchDetectedSubstring() {
        let text = "见 docs/a/b.md 和 /tmp/c.txt 两个文件"
        let links = GaryxFilePathLinkDetector.detect(in: text)
        XCTAssertEqual(links.count, 2)
        for link in links {
            let start = text.index(text.startIndex, offsetBy: link.characterOffset)
            let end = text.index(start, offsetBy: link.characterCount)
            XCTAssertEqual(String(text[start..<end]), link.target)
        }
    }

    func testIgnoresBareUrlsAndDomains() {
        XCTAssertEqual(
            GaryxFilePathLinkDetector.detect(in: "see https://example.com/a/file.html here"),
            []
        )
        XCTAssertEqual(
            GaryxFilePathLinkDetector.detect(in: "see example.com/a/file.html here"),
            []
        )
    }

    func testIgnoresProseSlashesWithoutExtension() {
        XCTAssertEqual(GaryxFilePathLinkDetector.detect(in: "TCP/IP and/or A/B testing"), [])
        XCTAssertEqual(GaryxFilePathLinkDetector.detect(in: "src/main without extension"), [])
    }

    func testIgnoresHomeRelativePaths() {
        XCTAssertEqual(GaryxFilePathLinkDetector.detect(in: "open ~/notes/today.md"), [])
    }

    func testStopsBeforeLineColumnSuffix() {
        let links = GaryxFilePathLinkDetector.detect(in: "error at src/lib/mod.rs:42:7")
        XCTAssertEqual(links.map(\.target), ["src/lib/mod.rs"])
    }

    func testLinkURLRoundTrip() throws {
        let target = "/Users/test/repos/garyx/docs/design/mobile gateway.html"
        let url = try XCTUnwrap(GaryxFilePathLinkDetector.linkURL(forTarget: target))
        XCTAssertEqual(url.scheme, GaryxFilePathLinkDetector.linkScheme)
        XCTAssertEqual(GaryxFilePathLinkDetector.target(from: url), target)
    }

    func testTargetFromForeignURLIsNil() throws {
        let url = try XCTUnwrap(URL(string: "https://example.com?target=%2Fa%2Fb.md"))
        XCTAssertNil(GaryxFilePathLinkDetector.target(from: url))
    }
}
