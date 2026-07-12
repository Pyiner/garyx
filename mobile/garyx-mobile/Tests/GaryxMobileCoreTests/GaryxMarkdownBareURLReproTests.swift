import Foundation
import XCTest

@testable import GaryxMobileCore

final class GaryxMarkdownBareURLReproTests: XCTestCase {
    func testCapturedAssistantBulletURLsAreLinks() {
        // Structurally equivalent, public-safe fixture for the captured
        // assistant render-state message that triggered TASK-2164. Both
        // visible destinations were inside inline-code delimiters.
        let assistantBody = """
        访问地址：

        - Home 内网：`http://192.0.2.10:8789/`
        - 通过现有解析规则：`http://service.example.com:8789/`
        """

        let renderedTargets = assistantBody
            .split(separator: "\n")
            .compactMap { line -> String? in
                let trimmed = line.trimmingCharacters(in: .whitespaces)
                guard trimmed.hasPrefix("- ") else { return nil }
                return String(trimmed.dropFirst(2))
            }
            .flatMap { linkSnapshots(in: render($0)).map(\.target) }

        XCTAssertEqual(
            renderedTargets,
            [
                "http://192.0.2.10:8789/",
                "http://service.example.com:8789/",
            ]
        )
    }

    func testDetectsBareHTTPAndHTTPSURLs() {
        let rendered = render(
            "HTTP http://example.com/a and HTTPS https://example.com/b?q=one#top"
        )

        XCTAssertEqual(
            linkSnapshots(in: rendered),
            [
                LinkSnapshot(text: "http://example.com/a", target: "http://example.com/a"),
                LinkSnapshot(
                    text: "https://example.com/b?q=one#top",
                    target: "https://example.com/b?q=one#top"
                ),
            ]
        )
    }

    func testDetectsIPv4AndDomainPortsWithPaths() {
        let rendered = render(
            "http://192.0.2.10:8789/ https://api.example.com:9443/v1/items?q=one"
        )

        XCTAssertEqual(
            linkSnapshots(in: rendered).map(\.target),
            [
                "http://192.0.2.10:8789/",
                "https://api.example.com:9443/v1/items?q=one",
            ]
        )
    }

    func testRepairsFoundationAutolinkThatSwallowsChinesePunctuationAndProse() {
        let rendered = render("前缀http://example.com:8789/，然后继续。")

        XCTAssertEqual(
            linkSnapshots(in: rendered),
            [
                LinkSnapshot(
                    text: "http://example.com:8789/",
                    target: "http://example.com:8789/"
                )
            ]
        )
    }

    func testKeepsASCIIAndChinesePunctuationOutsideURLs() {
        let rendered = render(
            "(https://example.com/path). 再看https://example.com/next！"
        )

        XCTAssertEqual(
            linkSnapshots(in: rendered),
            [
                LinkSnapshot(
                    text: "https://example.com/path",
                    target: "https://example.com/path"
                ),
                LinkSnapshot(
                    text: "https://example.com/next",
                    target: "https://example.com/next"
                ),
            ]
        )
    }

    func testKeepsChineseCharactersInIRIPath() {
        let rendered = render("http://example.com/路径说明")

        XCTAssertEqual(
            linkSnapshots(in: rendered),
            [
                LinkSnapshot(
                    text: "http://example.com/路径说明",
                    target: "http://example.com/%E8%B7%AF%E5%BE%84%E8%AF%B4%E6%98%8E"
                )
            ]
        )
    }

    func testRepairsIDNAutolinkWithoutSwallowingChinesePunctuation() {
        let rendered = render("http://例子.测试/x，然后")

        XCTAssertEqual(
            linkSnapshots(in: rendered),
            [
                LinkSnapshot(
                    text: "http://例子.测试/x",
                    target: "http://xn--fsqu00a.xn--0zwm56d/x"
                )
            ]
        )
    }

    func testLongURLRemainsOneLinkBeforeVisualWrapping() {
        let url = "https://example.com:9443/" + String(repeating: "long-segment/", count: 12) + "end"
        let rendered = render("气泡内的长地址：\(url)，继续")

        XCTAssertEqual(
            linkSnapshots(in: rendered),
            [LinkSnapshot(text: url, target: url)]
        )
    }

    func testPreservesExplicitMarkdownLink() {
        let rendered = render("[Open docs](https://example.com/docs)")

        XCTAssertEqual(
            linkSnapshots(in: rendered),
            [
                LinkSnapshot(
                    text: "Open docs",
                    target: "https://example.com/docs"
                )
            ]
        )
    }

    func testInlineCodeURLKeepsCodeIntentWhileBecomingLink() throws {
        let rendered = render("Use `https://example.com:9443/path` now")
        let linkRun = try XCTUnwrap(rendered.runs.first(where: { $0.link != nil }))

        XCTAssertEqual(
            String(rendered[linkRun.range].characters),
            "https://example.com:9443/path"
        )
        XCTAssertEqual(linkRun.link?.absoluteString, "https://example.com:9443/path")
        XCTAssertTrue(linkRun.inlinePresentationIntent?.contains(.code) == true)
    }

    func testFencedCodeBlockURLIsNotLinked() {
        let rendered = render(
            """
            Before

            ```text
            https://example.com/inside-code
            ```

            After
            """
        )

        XCTAssertEqual(linkSnapshots(in: rendered), [])
    }

    func testRecognizesWWWWithFoundationCompatibleTarget() {
        let rendered = render("Visit www.example.com/docs now")

        XCTAssertEqual(
            linkSnapshots(in: rendered),
            [
                LinkSnapshot(
                    text: "www.example.com/docs",
                    target: "http://www.example.com/docs"
                )
            ]
        )
    }

    func testRejectsFilenameTLDsAndBareDomainsInProseAndInlineCode() {
        let rendered = render(
            "Edit main.rs and README.md, then inspect api.example.com and `worker.rs`."
        )

        XCTAssertEqual(linkSnapshots(in: rendered), [])
    }

    func testLeavesFoundationEmailLinkUntouchedWithoutAddingOtherMatches() {
        let rendered = render("Email test@example.com about main.rs")

        XCTAssertEqual(
            linkSnapshots(in: rendered),
            [
                LinkSnapshot(
                    text: "test@example.com",
                    target: "mailto:test@example.com"
                )
            ]
        )
    }

    func testKeepsBareFilePathLinksOnSharedCorePath() {
        let rendered = render(
            "Open docs/design/notes.md and /Users/test/notes.txt",
            linkifyFilePaths: true
        )
        let snapshots = linkSnapshots(in: rendered)

        XCTAssertEqual(snapshots.map(\.text), ["docs/design/notes.md", "/Users/test/notes.txt"])
        XCTAssertEqual(
            snapshots.compactMap { URL(string: $0.target) }.compactMap(GaryxFilePathLinkDetector.target),
            ["docs/design/notes.md", "/Users/test/notes.txt"]
        )
    }

    private func render(
        _ markdown: String,
        linkifyFilePaths: Bool = false
    ) -> AttributedString {
        GaryxMarkdownAttributedStringRenderer.attributedString(
            from: markdown,
            linkifyFilePaths: linkifyFilePaths
        )
    }

    private func linkSnapshots(in attributed: AttributedString) -> [LinkSnapshot] {
        attributed.runs.compactMap { run in
            guard let link = run.link else { return nil }
            return LinkSnapshot(
                text: String(attributed[run.range].characters),
                target: link.absoluteString
            )
        }
    }
}

private struct LinkSnapshot: Equatable {
    let text: String
    let target: String
}
