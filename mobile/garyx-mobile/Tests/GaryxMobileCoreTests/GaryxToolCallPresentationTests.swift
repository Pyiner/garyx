import XCTest
@testable import GaryxMobileCore

final class GaryxToolCallPresentationTests: XCTestCase {
    func testCommandRowUsesDescriptionWithRanVerb() {
        let rows = GaryxToolCallPresentation.listRows(from: [
            entry(toolName: "bash", title: "Bash", summaryText: "查看交卷报错"),
        ])
        XCTAssertEqual(rows.map(\.verb), ["Ran"])
        XCTAssertEqual(rows.map(\.detail), ["查看交卷报错"])
        XCTAssertEqual(rows.map(\.icon), [.command])
    }

    func testCommandRowPrefersInputLabelOverCommandAndOutputSummary() {
        let rows = GaryxToolCallPresentation.listRows(from: [
            entry(
                toolName: "exec_command",
                title: "Command",
                inputText: "{\"label\": \"Run mobile tests\", \"command\": \"swift test\"}",
                summaryText: "Executed 298 tests"
            ),
        ])
        XCTAssertEqual(rows.map(\.verb), ["Ran"])
        XCTAssertEqual(rows.map(\.detail), ["Run mobile tests"])
    }

    func testCommandRowReadsWrappedToolInputDescription() {
        let rows = GaryxToolCallPresentation.listRows(from: [
            entry(
                toolName: "bash",
                title: "Bash",
                inputText: #"{"input":{"description":"Check public DNS","command":"dig example.test"},"tool":"Bash"}"#,
                summaryText: "example.test. 60 IN A 192.0.2.10"
            ),
        ])
        XCTAssertEqual(rows.map(\.verb), ["Ran"])
        XCTAssertEqual(rows.map(\.detail), ["Check public DNS"])
    }

    func testCommandRowFallsBackToCommandInputInsteadOfOutputSummary() {
        let rows = GaryxToolCallPresentation.listRows(from: [
            entry(
                toolName: "exec_command",
                title: "Command",
                inputText: "{\"command\": \"bash -lc 'swift test 2>&1'\"}",
                summaryText: "Executed 298 tests"
            ),
        ])
        XCTAssertEqual(rows.map(\.verb), ["Ran"])
        XCTAssertEqual(rows.map(\.detail), ["swift test"])
    }

    func testCommandRowReadsWrappedToolInputCommandInsteadOfOutputSummary() {
        let rows = GaryxToolCallPresentation.listRows(from: [
            entry(
                toolName: "bash",
                title: "Bash",
                inputText: #"{"input":{"command":"bash -lc 'swift test 2>&1'"},"tool":"Bash"}"#,
                summaryText: "Executed 298 tests"
            ),
        ])
        XCTAssertEqual(rows.map(\.verb), ["Ran"])
        XCTAssertEqual(rows.map(\.detail), ["swift test"])
    }

    func testCommandRowUsesRawInputFromDebugSnapshotInsteadOfOutputSummary() {
        let rows = GaryxToolCallPresentation.listRows(from: [
            entry(
                toolName: "exec_command",
                title: "Bash",
                inputText: "swift test",
                summaryText: "swift test passed"
            ),
        ])
        XCTAssertEqual(rows.map(\.verb), ["Ran"])
        XCTAssertEqual(rows.map(\.detail), ["swift test"])
    }

    func testGenericToolRowPrefersInputNameOverOutputSummary() {
        let rows = GaryxToolCallPresentation.listRows(from: [
            entry(
                toolName: "custom_tool",
                title: "Custom Tool",
                inputText: "{\"name\": \"Fetch candidate list\"}",
                summaryText: "Returned 40 rows"
            ),
        ])
        XCTAssertEqual(rows.map(\.detail), ["Fetch candidate list"])
    }

    func testRunningCommandRowShowsRunningVerb() {
        let rows = GaryxToolCallPresentation.listRows(from: [
            entry(toolName: "bash", title: "Bash", summaryText: "等待点词测试结果", status: .running),
        ])
        XCTAssertEqual(rows.map(\.verb), ["Running"])
        XCTAssertTrue(rows[0].isRunning)
    }

    func testReadRowShowsFullPathWithEyeIcon() {
        let rows = GaryxToolCallPresentation.listRows(from: [
            entry(
                toolName: "read",
                title: "Read",
                summaryText: "read rp-graded.png",
                primaryPath: "/tmp/voyage-shots/rp-graded.png"
            ),
        ])
        XCTAssertEqual(rows.map(\.verb), ["Read"])
        XCTAssertEqual(rows.map(\.detail), ["/tmp/voyage-shots/rp-graded.png"])
        XCTAssertEqual(rows.map(\.icon), [.read])
    }

    func testCodexViewImageCountsAsRead() {
        let rows = GaryxToolCallPresentation.listRows(from: [
            entry(toolName: "view_image", title: "Image", primaryPath: "shots/result.png"),
        ])
        XCTAssertEqual(rows.map(\.verb), ["Read"])
        XCTAssertEqual(rows.map(\.icon), [.read])
    }

    func testGenericToolRowFallsBackToUsedTitle() {
        let rows = GaryxToolCallPresentation.listRows(from: [
            entry(toolName: "todowrite", title: "Todowrite", summaryText: "3 todo items"),
        ])
        XCTAssertEqual(rows.map(\.verb), ["Used Todowrite"])
        XCTAssertEqual(rows.map(\.detail), ["3 todo items"])
    }

    func testImageRefsCollectReadViewImageAndWrittenImages() {
        let refs = GaryxToolCallPresentation.imageRefs(from: [
            entry(toolName: "read", title: "Read", primaryPath: "/tmp/a.png"),
            entry(toolName: "view_image", title: "Image", primaryPath: "/tmp/b.JPG"),
            entry(toolName: "write", title: "Write", primaryPath: "/tmp/generated.webp"),
            entry(toolName: "read", title: "Read", primaryPath: "/tmp/notes.md"),
            entry(toolName: "bash", title: "Bash", summaryText: "ls"),
        ])
        XCTAssertEqual(refs.map(\.path), ["/tmp/a.png", "/tmp/b.JPG", "/tmp/generated.webp"])
        XCTAssertEqual(refs.first?.fileName, "a.png")
    }

    func testImageRefsDeduplicateRepeatedPaths() {
        let refs = GaryxToolCallPresentation.imageRefs(from: [
            entry(id: "t1", toolName: "read", title: "Read", primaryPath: "/tmp/a.png"),
            entry(id: "t2", toolName: "read", title: "Read", primaryPath: "/tmp/a.png"),
        ])
        XCTAssertEqual(refs.count, 1)
    }

    func testCommandDetailShowsBareCommandAndRunningOutput() {
        let detail = GaryxToolCallPresentation.detail(for: entry(
            toolName: "bash",
            title: "Bash",
            inputText: "{\"command\": \"until [ -s output ]; do sleep 1; done\", \"description\": \"wait\"}",
            resultText: nil,
            status: .running
        ))
        XCTAssertEqual(detail.title, "Bash")
        XCTAssertTrue(detail.isRunning)
        XCTAssertEqual(detail.sections.map(\.label), ["Command", "Output"])
        XCTAssertEqual(
            detail.sections[0].content,
            .codeCard("until [ -s output ]; do sleep 1; done")
        )
        XCTAssertEqual(detail.sections[1].content, .codeCard("Running…"))
    }

    func testCommandDetailShowsBareCommandFromWrappedToolInput() {
        let detail = GaryxToolCallPresentation.detail(for: entry(
            toolName: "bash",
            title: "Bash",
            inputText: #"{"input":{"description":"Run mobile tests","command":"swift test"},"tool":"Bash"}"#,
            resultText: "Test Suite passed"
        ))
        XCTAssertEqual(detail.sections.map(\.label), ["Command", "Output"])
        XCTAssertEqual(detail.sections[0].content, .codeCard("swift test"))
    }

    func testReadDetailOfImageShowsImagePreviewNotBase64() {
        let detail = GaryxToolCallPresentation.detail(for: entry(
            toolName: "read",
            title: "Read",
            inputText: "{\"file_path\": \"/tmp/a.png\"}",
            resultText: "iVBORw0KGgoAAAANSUh…",
            primaryPath: "/tmp/a.png"
        ))
        XCTAssertEqual(detail.sections.map(\.label), ["File", "Content"])
        XCTAssertEqual(detail.sections[0].content, .plainMonospace("/tmp/a.png"))
        XCTAssertEqual(detail.sections[1].content, .imagePreview("/tmp/a.png"))
    }

    func testReadDetailOfTextFileKeepsResultCard() {
        let detail = GaryxToolCallPresentation.detail(for: entry(
            toolName: "read",
            title: "Read",
            inputText: "{\"file_path\": \"/tmp/notes.md\"}",
            resultText: "# Notes",
            primaryPath: "/tmp/notes.md"
        ))
        XCTAssertEqual(detail.sections.map(\.label), ["File", "Result"])
        XCTAssertEqual(detail.sections[1].content, .codeCard("# Notes"))
    }

    func testWriteDetailOfImagePathShowsImagePreview() {
        let detail = GaryxToolCallPresentation.detail(for: entry(
            toolName: "write",
            title: "Write",
            inputText: "{\"file_path\": \"/tmp/generated.png\"}",
            primaryPath: "/tmp/generated.png"
        ))
        XCTAssertEqual(detail.sections.map(\.label), ["File", "Content"])
        XCTAssertEqual(detail.sections[1].content, .imagePreview("/tmp/generated.png"))
    }

    func testEditDetailRendersOldNewStringsAsDiff() {
        let detail = GaryxToolCallPresentation.detail(for: entry(
            toolName: "edit",
            title: "Edit",
            inputText: "{\"file_path\": \"/src/App.swift\", \"old_string\": \"let a = 1\\nlet b = 2\", \"new_string\": \"let a = 3\"}",
            resultText: "ok",
            primaryPath: "/src/App.swift"
        ))
        XCTAssertEqual(detail.sections.map(\.label), ["File", "Output"])
        guard case .diff(let lines) = detail.sections[1].content else {
            return XCTFail("Expected a diff section for an edit call")
        }
        XCTAssertEqual(lines.map(\.kind), [.removed, .removed, .added])
        XCTAssertEqual(lines.map(\.text), ["let a = 1", "let b = 2", "let a = 3"])
    }

    func testWriteDetailRendersContentAsAllAddedDiff() {
        let detail = GaryxToolCallPresentation.detail(for: entry(
            toolName: "write",
            title: "Write",
            inputText: "{\"file_path\": \"/src/New.swift\", \"content\": \"import Foundation\\nstruct New {}\"}",
            primaryPath: "/src/New.swift"
        ))
        guard case .diff(let lines) = detail.sections[1].content else {
            return XCTFail("Expected a diff section for a write call")
        }
        XCTAssertEqual(lines.map(\.kind), [.added, .added])
        XCTAssertEqual(lines.map(\.text), ["import Foundation", "struct New {}"])
    }

    func testPatchStyleInputParsesPlusMinusLines() {
        let lines = GaryxToolCallPresentation.diffLines(
            input: nil,
            inputText: "@@ context @@\n-old line\n+new line\n+second new\n unchanged"
        )
        XCTAssertEqual(lines?.map(\.kind), [.context, .removed, .added, .added, .context])
    }

    func testPlainTextInputIsNotMistakenForDiff() {
        XCTAssertNil(GaryxToolCallPresentation.diffLines(
            input: nil,
            inputText: "ls -la\necho done"
        ))
    }

    func testGroupSummaryCountsCommandsReadsAndEdits() {
        let group = GaryxMobileToolTraceGroup(entries: [
            entry(toolName: "bash", title: "Bash", summaryText: "build"),
            entry(toolName: "bash", title: "Bash", summaryText: "test"),
            entry(toolName: "read", title: "Read", primaryPath: "/tmp/a.png"),
            entry(toolName: "read", title: "Read", primaryPath: "/tmp/b.png"),
            entry(toolName: "edit", title: "Edit", primaryPath: "/src/App.swift"),
        ])
        XCTAssertEqual(group.summary, "Ran 2 commands, read 2 files, edited 1 file")
    }

    func testGroupSummaryReadOnlyGroup() {
        let group = GaryxMobileToolTraceGroup(entries: [
            entry(toolName: "read", title: "Read", primaryPath: "/tmp/a.png"),
        ])
        XCTAssertEqual(group.summary, "Read 1 file")
    }

    private func entry(
        id: String = UUID().uuidString,
        toolName: String,
        title: String,
        inputText: String? = nil,
        resultText: String? = nil,
        summaryText: String? = nil,
        status: GaryxMobileToolTraceStatus = .completed,
        isError: Bool = false,
        primaryPath: String? = nil
    ) -> GaryxMobileToolTraceEntry {
        GaryxMobileToolTraceEntry(
            id: id,
            toolUseId: id,
            parentToolUseId: nil,
            toolName: toolName,
            title: title,
            inputText: inputText,
            resultText: resultText,
            summaryText: summaryText,
            inputLabel: "Call",
            resultLabel: "Result",
            status: status,
            isError: isError,
            timestamp: nil,
            primaryPathBadge: primaryPath.map { ($0 as NSString).lastPathComponent },
            primaryPath: primaryPath
        )
    }
}
