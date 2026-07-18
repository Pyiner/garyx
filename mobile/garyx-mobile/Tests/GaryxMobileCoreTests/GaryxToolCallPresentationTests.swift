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

    func testGenericToolRowUsesToolNameAsVerbWithoutUsedPrefix() {
        let rows = GaryxToolCallPresentation.listRows(from: [
            entry(toolName: "todowrite", title: "Todowrite", summaryText: "3 todo items"),
        ])
        XCTAssertEqual(rows.map(\.verb), ["Todowrite"])
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

    func testProjectionlessDetailDoesNotReparseLegacyRawPayload() {
        let legacyRawEntry = entry(
            toolName: "Edit",
            inputText: "{\"file_path\":\"/Users/test/repo/App.swift\",\"old_string\":\"a\",\"new_string\":\"b\"}",
            resultText: "Updated",
            primaryPath: "/Users/test/repo/App.swift"
        )

        XCTAssertNil(legacyRawEntry.fieldProjection)
        XCTAssertTrue(GaryxToolCallPresentation.detail(for: legacyRawEntry).sections.isEmpty)
    }

    func testProjectedDiffResolverComposesSegmentsInOrderAndPreservesWhitespace() throws {
        let toolUse = GaryxTranscriptMessage(
            index: 0,
            role: .toolUse,
            content: json(#"{"unified":" context\n+added\n-removed\n+++\n---\n \n+","new_only":"  \n","old_only":"\nold","old":" old \n","new":"\nnew"}"#)
        )
        let value: ([String]) -> GaryxRenderToolValueSelector = {
            GaryxRenderToolValueSelector(root: .content, path: $0)
        }
        let projection = GaryxRenderToolFieldProjection(
            kind: .fileEdit,
            diff: GaryxRenderToolDiffRecipe(
                source: .toolUse,
                segments: [
                    .unified(text: value(["unified"])),
                    .pair(old: nil, new: value(["new_only"])),
                    .pair(old: value(["old_only"]), new: nil),
                    .pair(old: value(["old"]), new: value(["new"])),
                ]
            )
        )

        let resolved = try XCTUnwrap(GaryxToolFieldProjectionResolver.resolve(
            projection,
            toolUse: toolUse,
            toolResult: nil
        ))
        XCTAssertEqual(resolved.diff.map(\.kind), [
            .context, .added, .removed, .context, .context, .context, .added,
            .added, .added, .removed, .removed, .removed, .removed, .added, .added,
        ])
        XCTAssertEqual(resolved.diff.map(\.text), [
            " context", "added", "removed", "+++", "---", " ", "",
            "  ", "", "", "old", " old ", "", "", "new",
        ])
    }

    func testProjectedDiffResolverUsesOnlyTheDeclaredSourceBody() throws {
        let toolUse = GaryxTranscriptMessage(
            index: 0,
            role: .toolUse,
            content: json(#"{"body":"+wrong source"}"#)
        )
        let toolResult = GaryxTranscriptMessage(
            index: 1,
            role: .toolResult,
            content: json(#"{"body":"+right source"}"#)
        )
        let projection = GaryxRenderToolFieldProjection(
            kind: .fileEdit,
            diff: GaryxRenderToolDiffRecipe(
                source: .toolResult,
                segments: [
                    .unified(text: GaryxRenderToolValueSelector(
                        root: .content,
                        path: ["body"]
                    )),
                ]
            )
        )

        let resolved = try XCTUnwrap(GaryxToolFieldProjectionResolver.resolve(
            projection,
            toolUse: toolUse,
            toolResult: toolResult
        ))
        XCTAssertEqual(resolved.diff.map(\.kind), [.added])
        XCTAssertEqual(resolved.diff.map(\.text), ["right source"])
    }

    func testProjectedDetailOrdersFileCallDiffResultAndCollapsesToPathTail() {
        let path = "/Users/test/repo/Sample.swift"
        let projection = GaryxResolvedToolFieldProjection(
            kind: .fileEdit,
            toolName: "Edit",
            summary: GaryxResolvedToolField(text: path, label: "File", format: .path),
            call: GaryxResolvedToolField(text: "replace_all=false", label: "Call", format: .code),
            diff: [GaryxToolCallDiffLine(id: 0, kind: .added, text: "let value = 2")],
            result: GaryxResolvedToolField(text: "Updated", label: "Result", format: .text),
            status: nil,
            exitCode: nil,
            durationMs: nil
        )
        let projectedEntry = entry(
            toolName: "Edit",
            primaryPath: path,
            primaryPathBadge: "repo/Sample.swift",
            fieldProjection: projection
        )

        XCTAssertEqual(
            GaryxToolCallPresentation.detail(for: projectedEntry).sections.map(\.label),
            ["File", "Call", "Diff", "Result"]
        )
        XCTAssertEqual(
            GaryxToolCallPresentation.listRows(from: [projectedEntry]).first?.detail,
            "repo/Sample.swift"
        )
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

    // Reproduction (state-level, real tool names from a thread transcript): an Edit
    // grouped with a non-file tool (Agent / TaskCreate / ToolSearch / Skill /
    // mcp__*). None classify as command/read/edit, so the second folds into the
    // generic "used N tools" — the unhelpful "Edited 1 file, used 1 tool" the user
    // circled. The summary should name the tool by its title instead.
    func testGroupSummaryNamesNonFileToolInsteadOfGenericUsedOneTool() {
        let group = GaryxMobileToolTraceGroup(entries: [
            entry(toolName: "Edit", title: "Edit", primaryPath: "/src/App.swift"),
            entry(toolName: "Agent"),
        ])
        XCTAssertEqual(group.summary, "Edited 1 file, used Agent")
    }

    func testGroupSummaryNamesTwoDistinctNonFileTools() {
        let group = GaryxMobileToolTraceGroup(entries: [
            entry(toolName: "TaskCreate"),
            entry(toolName: "ToolSearch"),
        ])
        XCTAssertEqual(group.entries.map(\.title), ["TaskCreate", "ToolSearch"])
        XCTAssertEqual(group.summary, "Used TaskCreate, ToolSearch")
    }

    func testGroupSummaryCapsManyDistinctNonFileToolNames() {
        let group = GaryxMobileToolTraceGroup(entries: [
            entry(toolName: "TaskCreate"),
            entry(toolName: "ToolSearch"),
            entry(toolName: "Agent"),
            entry(toolName: "Skill"),
        ])
        XCTAssertEqual(group.summary, "Used 4 tools")
    }

    func testExpandedRowVerbNamesNonFileToolDirectlyWithoutUsedPrefix() {
        // The expanded tool-list rows name the tool directly (no "Used"/"Using"
        // prefix); file/command verbs are unchanged.
        let rows = GaryxToolCallPresentation.listRows(from: [
            entry(toolName: "Agent", title: "Agent"),
            entry(toolName: "edit", title: "Edit", primaryPath: "/src/App.swift"),
            entry(toolName: "bash", title: "Bash", summaryText: "git status"),
        ])
        XCTAssertEqual(rows[0].verb, "Agent")
        XCTAssertEqual(rows[1].verb, "Edited")
        XCTAssertEqual(rows[2].verb, "Ran")
    }

    func testExpandedRowVerbForRunningNonFileToolIsJustTheName() {
        let rows = GaryxToolCallPresentation.listRows(from: [
            entry(toolName: "ToolSearch", status: .running),
        ])
        XCTAssertEqual(rows[0].verb, "ToolSearch")
    }

    private func json(_ raw: String) -> GaryxJSONValue {
        try! JSONDecoder().decode(GaryxJSONValue.self, from: Data(raw.utf8))
    }

    private func entry(
        id: String = UUID().uuidString,
        toolName: String,
        title: String? = nil,
        inputText: String? = nil,
        resultText: String? = nil,
        summaryText: String? = nil,
        status: GaryxMobileToolTraceStatus = .completed,
        isError: Bool = false,
        primaryPath: String? = nil,
        primaryPathBadge: String? = nil,
        fieldProjection: GaryxResolvedToolFieldProjection? = nil
    ) -> GaryxMobileToolTraceEntry {
        GaryxMobileToolTraceEntry(
            id: id,
            toolUseId: id,
            parentToolUseId: nil,
            toolName: toolName,
            title: title ?? GaryxMobileToolTraceEntry.title(for: toolName),
            inputText: inputText,
            resultText: resultText,
            summaryText: summaryText,
            inputLabel: "Call",
            resultLabel: "Result",
            status: status,
            isError: isError,
            timestamp: nil,
            primaryPathBadge: primaryPathBadge ?? primaryPath.map { ($0 as NSString).lastPathComponent },
            primaryPath: primaryPath,
            fieldProjection: fieldProjection
        )
    }
}
