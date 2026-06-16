import XCTest
@testable import GaryxMobileCore

final class GaryxMobileToolTraceBuilderTests: XCTestCase {
    func testCommittedBuilderDropsOrphanToolResult() {
        let messages = GaryxMobileTranscriptMapper.mobileMessages(from: [
            GaryxTranscriptMessage(index: 1, role: .assistant, text: "Done."),
            GaryxTranscriptMessage(
                index: 2,
                role: .toolResult,
                content: json(#"{"result":{"text":"orphan result"}}"#)
            ),
        ])

        XCTAssertEqual(messages.map(\.role), [.assistant])
        XCTAssertTrue(messages.filter { $0.role == .tool }.isEmpty)
    }

    func testCommittedBuilderHidesInternalToolUseItems() {
        let messages = GaryxMobileTranscriptMapper.mobileMessages(from: [
            GaryxTranscriptMessage(index: 1, role: .assistant, text: "Working."),
            GaryxTranscriptMessage(
                index: 2,
                role: .toolUse,
                content: json(#"{"type":"fileChange","path":"/tmp/App.swift"}"#)
            ),
            GaryxTranscriptMessage(
                index: 3,
                role: .toolUse,
                content: json(#"{"type":"contextCompaction","summary":"compacted"}"#)
            ),
        ])

        XCTAssertEqual(messages.map(\.role), [.assistant])
        XCTAssertTrue(messages.filter { $0.role == .tool }.isEmpty)
        XCTAssertFalse(messages.contains { $0.text.localizedCaseInsensitiveContains("Filechange") })
        XCTAssertFalse(messages.contains { $0.text.localizedCaseInsensitiveContains("contextCompaction") })
    }

    func testDerivedToolTitlesPreserveCamelCaseToolNames() throws {
        XCTAssertEqual(GaryxMobileToolTraceEntry.title(for: "TaskCreate"), "TaskCreate")

        let messages = GaryxMobileTranscriptMapper.mobileMessages(from: [
            GaryxTranscriptMessage(
                index: 1,
                role: .toolUse,
                content: json(#"{"tool":"TaskCreate","input":{"title":"Review"}}"#)
            ),
            GaryxTranscriptMessage(
                index: 2,
                role: .toolUse,
                content: json(#"{"tool":"ToolSearch","input":{"query":"review"}}"#)
            ),
        ])

        let group = try XCTUnwrap(messages.first?.toolTraceGroup)
        XCTAssertEqual(group.entries.map(\.title), ["TaskCreate", "ToolSearch"])
        XCTAssertEqual(group.summary, "Used TaskCreate, ToolSearch")
    }

    private func json(_ raw: String) -> GaryxJSONValue {
        try! JSONDecoder().decode(GaryxJSONValue.self, from: Data(raw.utf8))
    }
}
