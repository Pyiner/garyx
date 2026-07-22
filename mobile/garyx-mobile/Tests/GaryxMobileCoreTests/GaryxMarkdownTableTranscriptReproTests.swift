@testable import GaryxMobileCore
import XCTest

final class GaryxMarkdownTableTranscriptReproTests: XCTestCase {
    private let openingText = "我查一下远端 main 的提交时间线和上一轮 TestFlight 实际构建的 commit，对比就清楚了。"
    private let finalText = "情况就是上面表格那样。现在继续等 #TASK-2609 完工通知，到了我验收 → 合 main → push → 发 TestFlight，一次带齐。"

    /// Sanitized from the canonical seq 103...116 transcript records and the
    /// corresponding live render_state. The two assistant bodies are preserved
    /// byte-for-byte; unrelated tool payloads and identifiers are anonymized.
    ///
    /// This pins the actual failure boundary: the client receives two ordinary
    /// string bodies, neither of which contains the table that the final body
    /// refers to. The final_message ref resolves to seq 116 without losing text.
    func testCapturedTurnHasNoTableBeforeMarkdownParsing() throws {
        let capture = try loadCapture()
        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 102, replayScope: .resume)
        let result = processor.processRenderFrame(capture, threadId: capture.threadId)

        XCTAssertNil(result.reconnect)
        guard case let .applyCommittedMessages(committed) = try XCTUnwrap(result.actions.first),
              case let .applyRenderSnapshot(snapshot) = try XCTUnwrap(result.actions.last) else {
            return XCTFail("captured frame must apply committed bodies before render_state")
        }

        let assistantMessages = committed.filter { $0.role == .assistant }
        XCTAssertEqual(assistantMessages.map(\.index), [105, 115])
        XCTAssertEqual(assistantMessages.map(\.text), [openingText, finalText])
        XCTAssertEqual(
            assistantMessages.map(\.content),
            [.string(openingText), .string(finalText)],
            "each committed assistant body is one String, not a segmented content array"
        )

        let mobileMessages = GaryxMobileTranscriptMapper.mobileMessages(from: committed)
        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: mobileMessages,
            transcriptMessages: committed
        )
        let mappedFinal = try finalMessage(in: rows)
        XCTAssertEqual(mappedFinal.historyIndex, 115)
        XCTAssertEqual(mappedFinal.text, finalText)

        let parsed = GaryxMarkdownBlockParser.blocks(from: mappedFinal.text)
        XCTAssertEqual(parsed.map(\.kind), [.markdown(finalText)])
        XCTAssertEqual(parsed.tableCount, 0)
        XCTAssertFalse(
            assistantMessages.map(\.text).joined(separator: "\n").contains("|"),
            "the supposed pipe table is absent even after joining every assistant string in the turn"
        )
    }

    /// Control for parser and render-ref mapping. Replacing only seq 116's body
    /// with the reported table draft makes the same final_message ref preserve
    /// the whole String and the existing parser produce a 3-column table.
    func testReportedTableDraftSurvivesTheSameMappingPath() throws {
        let capture = try loadCapture()
        var processor = GatewayStreamFrameProcessor()
        processor.resetConnection(afterSeq: 102, replayScope: .resume)
        let result = processor.processRenderFrame(capture, threadId: capture.threadId)

        guard case let .applyCommittedMessages(captured) = try XCTUnwrap(result.actions.first),
              case let .applyRenderSnapshot(snapshot) = try XCTUnwrap(result.actions.last) else {
            return XCTFail("captured frame must apply committed bodies before render_state")
        }

        var withTable = captured
        let finalIndex = try XCTUnwrap(withTable.firstIndex { $0.index == 115 })
        withTable[finalIndex].text = reportedTableDraft
        withTable[finalIndex].content = .string(reportedTableDraft)

        let rows = GaryxMobileRenderStateMapper.rows(
            snapshot: snapshot,
            messages: GaryxMobileTranscriptMapper.mobileMessages(from: withTable),
            transcriptMessages: withTable
        )
        let mappedFinal = try finalMessage(in: rows)
        XCTAssertEqual(mappedFinal.text, reportedTableDraft)

        let parsed = GaryxMarkdownBlockParser.blocks(from: mappedFinal.text)
        XCTAssertEqual(parsed.tableCount, 1)
        let tables = parsed.compactMap(\.table)
        XCTAssertEqual(tables.count, 1)
        let table = try XCTUnwrap(tables.first)
        XCTAssertEqual(table.columns.map { $0.title }, [
            "改动",
            "合入远端时间",
            "在上一轮 TestFlight（18:10，构建 18:06 的 `50731eff8`）里吗",
        ])
        XCTAssertEqual(table.rows.count, 6)
        XCTAssertEqual(table.rows.map { $0.count }, Array(repeating: 3, count: 6))
        XCTAssertEqual(table.rows[4], ["**Provider 卡片统一**", "今天 **19:03**", "❌ 晚了约 1 小时"])
    }

    private func loadCapture() throws -> GaryxThreadRenderFrame {
        let url = try XCTUnwrap(
            Bundle.module.url(
                forResource: "task-2610-markdown-table-frame",
                withExtension: "json",
                subdirectory: "Fixtures"
            )
        )
        return try JSONDecoder().decode(GaryxThreadRenderFrame.self, from: Data(contentsOf: url))
    }

    private func finalMessage(in rows: [GaryxMobileTurnRow]) throws -> GaryxMobileMessage {
        XCTAssertEqual(rows.count, 1)
        let row = try XCTUnwrap(rows.first)
        XCTAssertEqual(row.activityRows.count, 1)
        let activity = try XCTUnwrap(row.activityRows.first)
        guard case let .turn(turn) = activity else {
            throw ReproError.expectedAgentTurn
        }
        return try XCTUnwrap(turn.finalBlock?.message)
    }

    private var reportedTableDraft: String {
        """
        远端时间线如下：

        | 改动 | 合入远端时间 | 在上一轮 TestFlight（18:10，构建 18:06 的 `50731eff8`）里吗 |
        |---|---|---|
        | composer Worktree 不可选修复 | 今天 02:00 | ✅ 在（凌晨 04:20 的 build 就带上了） |
        | 短线程首条消息跳位修复 | 今天 03:17 | ✅ 在 |
        | teardown 崩溃修复（收敛合并） | 今天 14:48 | ✅ 在 |
        | Agent 编辑页头像按钮样式修复 | 今天 **18:12** | ❌ 晚了 6 分钟 |
        | **Provider 卡片统一** | 今天 **19:03** | ❌ 晚了约 1 小时 |
        | Workspace runtime 行对齐 | 今天 **19:41** | ❌ |

        表格之后的普通段落。
        """
    }
}

private enum ReproError: Error {
    case expectedAgentTurn
}

private extension Array where Element == GaryxMarkdownParsedBlock {
    var tableCount: Int {
        compactMap(\.table).count
    }
}

private extension GaryxMarkdownParsedBlock {
    var table: GaryxMarkdownParsedTable? {
        guard case let .table(table) = kind else { return nil }
        return table
    }
}
