import XCTest
@testable import GaryxMobileCore

final class GaryxMobileToolSummaryFormattingTests: XCTestCase {
    func testSafeToolSummaryUsesFirstNonEmptyLineAndSuppressesJsonObjects() {
        XCTAssertEqual(GaryxMobileToolSummaryFormatter.safeSummary("\n  first line\nsecond line"), "first line")
        XCTAssertNil(GaryxMobileToolSummaryFormatter.safeSummary(""))
        XCTAssertNil(GaryxMobileToolSummaryFormatter.safeSummary("{"))
        XCTAssertNil(GaryxMobileToolSummaryFormatter.safeSummary("{\"key\":\"value\"}"))
        XCTAssertEqual(GaryxMobileToolSummaryFormatter.safeSummary("[1,2]"), "[1,2]")
    }

    func testPathTailKeepsLastTwoPathSegments() {
        XCTAssertEqual(GaryxMobileToolSummaryFormatter.pathTail("/workspace/project-alpha/Sources/File.swift"), "Sources/File.swift")
        XCTAssertEqual(GaryxMobileToolSummaryFormatter.pathTail("C:\\workspace\\project-alpha\\File.swift"), "project-alpha/File.swift")
        XCTAssertEqual(GaryxMobileToolSummaryFormatter.pathTail("relative-file.swift"), "relative-file.swift")
    }

    func testShellSummaryStripsLaunchersQuotesNoiseAndWhitespace() {
        XCTAssertEqual(GaryxMobileToolSummaryFormatter.shellSummary("bash -lc 'git   status --short 2>&1'"), "git status --short")
        XCTAssertEqual(GaryxMobileToolSummaryFormatter.shellSummary("/bin/zsh -lc \"swift    test\""), "swift test")
        XCTAssertEqual(GaryxMobileToolSummaryFormatter.shellSummary("  npm   run   build  "), "npm run build")
    }

    func testSingleLineTruncationRespectsLimit() {
        XCTAssertEqual(GaryxMobileToolSummaryFormatter.singleLineTruncated("abcdef", limit: 4), "abc…")
        XCTAssertEqual(GaryxMobileToolSummaryFormatter.singleLineTruncated("abc", limit: 4), "abc")
    }
}
