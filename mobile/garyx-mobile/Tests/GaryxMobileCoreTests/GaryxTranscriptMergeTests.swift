import XCTest
@testable import GaryxMobileCore

final class GaryxTranscriptMergeTests: XCTestCase {
    func testRemoteMaterializationUsesSharedOriginIdentity() {
        let origin = "00000000-0000-0000-0000-000000000001"
        let local = [optimisticUser("origin:\(origin)", text: "hello", clientIntentId: origin)]
        let remote = [historyUser(0, text: "hello", clientIntentId: origin)]

        let merged = GaryxTranscriptMerge.mergedMessages(remote, withLocal: local)

        XCTAssertEqual(merged.count, 1)
        XCTAssertEqual(merged[0].id, "origin:\(origin)", "row identity must stay stable")
        XCTAssertEqual(merged[0].localState, .remoteFinal)
        XCTAssertEqual(merged[0].historyIndex, 0, "pagination math follows the committed row")
    }

    func testSameTextWithoutOriginDoesNotDedupOptimisticRows() {
        let local = [
            optimisticUser("origin:00000000-0000-0000-0000-000000000001", text: "same text"),
            optimisticUser("origin:00000000-0000-0000-0000-000000000002", text: "same text"),
        ]
        let remote = [historyUser(0, text: "same text")]

        let merged = GaryxTranscriptMerge.mergedMessages(remote, withLocal: local)

        XCTAssertEqual(
            merged.map(\.id),
            [
                "history:0",
                "origin:00000000-0000-0000-0000-000000000001",
                "origin:00000000-0000-0000-0000-000000000002",
            ]
        )
        XCTAssertEqual(merged[0].localState, .remoteFinal)
        XCTAssertEqual(merged[1].localState, .optimistic)
        XCTAssertEqual(merged[2].localState, .optimistic)
    }

    func testPreserveRemoteBeforeIndexKeepsOlderLoadedPages() {
        let olderPage = [
            historyUser(0, text: "old question"),
            historyAssistant(1, text: "old answer"),
        ]
        let latestPage = [
            historyUser(40, text: "new question"),
            historyAssistant(41, text: "new answer"),
        ]

        let merged = GaryxTranscriptMerge.mergedMessages(
            latestPage,
            withLocal: olderPage,
            preserveRemoteBeforeIndex: 40
        )

        XCTAssertEqual(merged.map(\.id), ["history:0", "history:1", "history:40", "history:41"])
    }

    func testFailedOptimisticSendSurvivesMerge() {
        var failed = optimisticUser("origin:00000000-0000-0000-0000-000000000009", text: "did not make it")
        failed.statusText = "gateway exploded"
        let remote = [historyUser(0, text: "older message")]

        let merged = GaryxTranscriptMerge.mergedMessages(remote, withLocal: [failed])

        XCTAssertEqual(merged.count, 2)
        XCTAssertEqual(merged[1].id, "origin:00000000-0000-0000-0000-000000000009")
        XCTAssertEqual(merged[1].statusText, "gateway exploded")
    }

    func testOptimisticSendEchoesImmediatelyAndDedupesWhenCommitted() {
        let origin = "00000000-0000-0000-0000-000000000010"
        let optimistic = optimisticUser(
            "origin:\(origin)",
            text: "continue",
            clientIntentId: origin
        )

        XCTAssertEqual(
            GaryxTranscriptMerge.mergedMessages([], withLocal: [optimistic]).map(\.id),
            ["origin:\(origin)"]
        )

        let committed = [historyUser(42, text: "continue", clientIntentId: origin)]
        let merged = GaryxTranscriptMerge.mergedMessages(committed, withLocal: [optimistic])

        XCTAssertEqual(merged.map(\.id), ["origin:\(origin)"])
        XCTAssertEqual(merged[0].localState, .remoteFinal)
    }

    func testEmptyRemoteKeepsLocalUntouched() {
        let local = [optimisticUser("origin:00000000-0000-0000-0000-000000000001", text: "hello")]
        XCTAssertEqual(GaryxTranscriptMerge.mergedMessages([], withLocal: local), local)
    }

    func testPreserveRemoteBeforeIndexKeepsOlderToolGroups() {
        let olderGroup = GaryxMobileToolTraceGroup(
            entries: [commandEntry("old-e1", toolUseId: "call-old", status: .completed)],
            live: false
        )
        let olderPage = [
            historyUser(0, text: "old question"),
            GaryxMobileMessage(
                id: "tool-group:old",
                role: .tool,
                text: olderGroup.summary,
                timestamp: nil,
                isStreaming: false,
                toolTraceGroup: olderGroup,
                localState: .remoteFinal,
                historyIndex: 1
            ),
            historyAssistant(2, text: "old answer"),
        ]
        let latestPage = [
            historyUser(40, text: "new question"),
            historyAssistant(41, text: "new answer"),
        ]

        let merged = GaryxTranscriptMerge.mergedMessages(
            latestPage,
            withLocal: olderPage,
            preserveRemoteBeforeIndex: 40
        )

        XCTAssertEqual(
            merged.map(\.id),
            ["history:0", "tool-group:old", "history:2", "history:40", "history:41"]
        )
    }

    private func historyUser(_ index: Int, text: String, clientIntentId: String? = nil) -> GaryxMobileMessage {
        GaryxMobileMessage(
            id: clientIntentId.map { "origin:\($0)" } ?? "history:\(index)",
            role: .user,
            text: text,
            timestamp: nil,
            isStreaming: false,
            clientIntentId: clientIntentId,
            localState: .remoteFinal,
            historyIndex: index
        )
    }

    private func historyAssistant(_ index: Int, text: String) -> GaryxMobileMessage {
        GaryxMobileMessage(
            id: "history:\(index)",
            role: .assistant,
            text: text,
            timestamp: nil,
            isStreaming: false,
            localState: .remoteFinal,
            historyIndex: index
        )
    }

    private func optimisticUser(_ id: String, text: String, clientIntentId: String? = nil) -> GaryxMobileMessage {
        GaryxMobileMessage(
            id: id,
            role: .user,
            text: text,
            timestamp: nil,
            isStreaming: false,
            clientIntentId: clientIntentId,
            localState: .optimistic
        )
    }

    private func commandEntry(
        _ id: String,
        toolUseId: String,
        status: GaryxMobileToolTraceStatus
    ) -> GaryxMobileToolTraceEntry {
        GaryxMobileToolTraceEntry(
            id: id,
            toolUseId: toolUseId,
            toolName: "commandExecution",
            title: "Command",
            inputText: "npm install",
            inputLabel: "Call",
            resultLabel: "Result",
            status: status,
            isError: false,
            timestamp: nil,
            primaryPathBadge: nil
        )
    }

}
