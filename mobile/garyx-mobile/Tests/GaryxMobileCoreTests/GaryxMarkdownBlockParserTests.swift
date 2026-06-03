import XCTest
@testable import GaryxMobileCore

final class GaryxMarkdownBlockParserTests: XCTestCase {
    func testEmptyAndWhitespaceInputFallsBackToMarkdownBlock() {
        XCTAssertEqual(GaryxMarkdownBlockParser.blocks(from: "").map(\.kind), [.markdown("")])
        XCTAssertEqual(GaryxMarkdownBlockParser.blocks(from: " \n\t ").map(\.kind), [.markdown(" \n\t ")])
    }

    func testNormalBacktickFenceParsesLanguageAndCode() {
        let blocks = GaryxMarkdownBlockParser.blocks(from: """
        Intro

        ```swift
        let value = 1
        ```

        Outro
        """)

        XCTAssertEqual(blocks.count, 3)
        XCTAssertEqual(blocks[0].kind, .markdown("Intro\n"))
        XCTAssertEqual(blocks[1].kind, .code(language: "swift", text: "let value = 1"))
        XCTAssertEqual(blocks[2].kind, .markdown("\nOutro"))
    }

    func testLongerBacktickFenceKeepsShorterInnerFenceAsCode() {
        let blocks = GaryxMarkdownBlockParser.blocks(from: """
        ````markdown
        # Prompt

        ```bash
        garyx task create --title "<title>"
        ```
        Done
        ````
        """)

        XCTAssertEqual(blocks.count, 1)
        guard case let .code(language, text) = blocks[0].kind else {
            return XCTFail("Expected one code block")
        }
        XCTAssertEqual(language, "markdown")
        XCTAssertTrue(text.contains("```bash"))
        XCTAssertTrue(text.contains("garyx task create"))
        XCTAssertTrue(text.contains("Done"))
    }

    func testOfficeCcEscapedInnerFenceStaysInsideOuterMarkdownFence() {
        let zeroWidthSpace = "\u{200B}"
        let blocks = GaryxMarkdownBlockParser.blocks(from: """
        ```markdown
        # You are Gary

        Delegate work with:

        \(zeroWidthSpace)```bash
        garyx task create --title "<title>" --body "<complete request>" --assignee <agent_id>
        \(zeroWidthSpace)```

        Your custom agents are listed by `garyx agent list`.

        ## Memory

        `/Users/test/.garyx/agents/gary/memory.md`
        ```
        """)

        XCTAssertEqual(blocks.count, 1)
        guard case let .code(language, text) = blocks[0].kind else {
            return XCTFail("Expected the escaped inner fence sample to stay in one code block")
        }
        XCTAssertEqual(language, "markdown")
        XCTAssertTrue(text.contains("\(zeroWidthSpace)```bash"))
        XCTAssertTrue(text.contains("garyx task create"))
        XCTAssertTrue(text.contains("Your custom agents"))
        XCTAssertTrue(text.contains("## Memory"))
    }

    func testZeroWidthSpacePrefixDoesNotOpenFence() {
        let zeroWidthSpace = "\u{200B}"
        let blocks = GaryxMarkdownBlockParser.blocks(from: """
        Intro
        \(zeroWidthSpace)```bash
        echo synthetic
        \(zeroWidthSpace)```
        Outro
        """)

        XCTAssertEqual(blocks.count, 1)
        guard case let .markdown(text) = blocks[0].kind else {
            return XCTFail("Expected escaped fences to remain markdown text")
        }
        XCTAssertTrue(text.contains("\(zeroWidthSpace)```bash"))
        XCTAssertTrue(text.contains("echo synthetic"))
    }

    func testClosingFenceRequiresOnlyAsciiWhitespaceAfterMarker() {
        let blocks = GaryxMarkdownBlockParser.blocks(from: """
        ```text
        not closed yet
        ``` trailing text
        still code
        ```
        """)

        XCTAssertEqual(blocks.count, 1)
        XCTAssertEqual(blocks[0].kind, .code(language: "text", text: "not closed yet\n``` trailing text\nstill code"))
    }

    func testIndentedFenceUsesAsciiSpaceAndTabOnly() {
        let blocks = GaryxMarkdownBlockParser.blocks(from: """
        \t```sh
        echo hi
           ```
        """)

        XCTAssertEqual(blocks.count, 1)
        XCTAssertEqual(blocks[0].kind, .code(language: "sh", text: "echo hi"))
    }

    func testCarriageReturnLineEndingsDoNotBreakFenceDetection() {
        let blocks = GaryxMarkdownBlockParser.blocks(from: "```swift\r\nlet value = 1\r\n```\r\n")

        XCTAssertEqual(blocks.count, 1)
        XCTAssertEqual(blocks[0].kind, .code(language: "swift", text: "let value = 1\r"))
    }

    func testTildeFenceUsesMatchingMarkerAndLength() {
        let blocks = GaryxMarkdownBlockParser.blocks(from: """
        ~~~~text
        ~~~
        still code
        ~~~~
        """)

        XCTAssertEqual(blocks.count, 1)
        XCTAssertEqual(blocks[0].kind, .code(language: "text", text: "~~~\nstill code"))
    }

    func testUnclosedFenceFlushesCodeAtEndOfFile() {
        let blocks = GaryxMarkdownBlockParser.blocks(from: """
        Before

        ```json
        {"ok": true}
        """)

        XCTAssertEqual(blocks.count, 2)
        XCTAssertEqual(blocks[0].kind, .markdown("Before\n"))
        XCTAssertEqual(blocks[1].kind, .code(language: "json", text: "{\"ok\": true}"))
    }

    func testStandaloneImageParsesAltAndSource() {
        let blocks = GaryxMarkdownBlockParser.blocks(from: """
        Before
        ![Example chart](https://example.com/chart.png "Chart")
        After
        """)

        XCTAssertEqual(blocks.count, 3)
        XCTAssertEqual(blocks[0].kind, .markdown("Before"))
        XCTAssertEqual(blocks[1].kind, .image(alt: "Example chart", source: "https://example.com/chart.png"))
        XCTAssertEqual(blocks[2].kind, .markdown("After"))
    }

    func testStandaloneImagePreservesFileSourcesWithSpaces() {
        let blocks = GaryxMarkdownBlockParser.blocks(from: """
        ![Local chart](<file:/workspace/project/My Chart.png>)
        ![Absolute chart](/workspace/project/My Chart.png)
        """)

        XCTAssertEqual(blocks.count, 2)
        XCTAssertEqual(blocks[0].kind, .image(alt: "Local chart", source: "file:/workspace/project/My Chart.png"))
        XCTAssertEqual(blocks[1].kind, .image(alt: "Absolute chart", source: "/workspace/project/My Chart.png"))
    }

    func testTableStopsBeforeFenceAndFenceParsesAsCode() {
        let blocks = GaryxMarkdownBlockParser.blocks(from: """
        | Name | Value |
        |---|---:|
        | Alpha | 1 |
        ```bash
        echo hi
        ```
        """)

        XCTAssertEqual(blocks.count, 2)
        guard case let .table(table) = blocks[0].kind else {
            return XCTFail("Expected table before fence")
        }
        XCTAssertEqual(table.columns.map(\.title), ["Name", "Value"])
        XCTAssertEqual(table.columns.map(\.alignment), [.leading, .trailing])
        XCTAssertEqual(table.rows, [["Alpha", "1"]])
        XCTAssertEqual(blocks[1].kind, .code(language: "bash", text: "echo hi"))
    }
}
