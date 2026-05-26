import XCTest
@testable import GaryxMobileCore

final class GaryxMobileToolSummaryFormattingTests: XCTestCase {
    func testSafeToolSummaryUsesFirstNonEmptyLineAndSuppressesJsonObjects() {
        XCTAssertEqual("\n  first line\nsecond line".garyxSafeToolSummary, "first line")
        XCTAssertNil("".garyxSafeToolSummary)
        XCTAssertNil("{".garyxSafeToolSummary)
        XCTAssertNil("{\"key\":\"value\"}".garyxSafeToolSummary)
    }

    func testPathTailKeepsLastTwoPathSegments() {
        XCTAssertEqual("/workspace/project-alpha/Sources/File.swift".garyxPathTail, "Sources/File.swift")
        XCTAssertEqual("C:\\workspace\\project-alpha\\File.swift".garyxPathTail, "project-alpha/File.swift")
        XCTAssertEqual("relative-file.swift".garyxPathTail, "relative-file.swift")
    }

    func testShellSummaryStripsLaunchersQuotesNoiseAndWhitespace() {
        XCTAssertEqual("bash -lc 'git   status --short 2>&1'".garyxShellSummary, "git status --short")
        XCTAssertEqual("/bin/zsh -lc \"swift    test\"".garyxShellSummary, "swift test")
        XCTAssertEqual("  npm   run   build  ".garyxShellSummary, "npm run build")
    }

    func testSingleLineTruncationRespectsLimit() {
        XCTAssertEqual("abcdef".garyxSingleLineTruncated(limit: 4), "abc…")
        XCTAssertEqual("abc".garyxSingleLineTruncated(limit: 4), "abc")
    }
}
