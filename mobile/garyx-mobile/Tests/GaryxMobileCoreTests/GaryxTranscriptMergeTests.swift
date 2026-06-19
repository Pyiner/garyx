import XCTest
@testable import GaryxMobileCore

final class GaryxTranscriptMergeTests: XCTestCase {
    func testRemoteMaterializationReusesLocalRowIdentity() {
        let local = [optimisticUser("local-user-1", text: "hello", clientIntentId: "mobile-1")]
        let remote = [historyUser(0, text: "hello", clientIntentId: "mobile-1")]

        let merged = GaryxTranscriptMerge.mergedMessages(remote, withLocal: local)

        XCTAssertEqual(merged.count, 1)
        XCTAssertEqual(merged[0].id, "local-user-1", "row identity must stay stable")
        XCTAssertEqual(merged[0].remoteId, "history:0")
        XCTAssertEqual(merged[0].localState, .remoteFinal)
        XCTAssertEqual(merged[0].historyIndex, 0, "pagination math follows the materialized row")
    }

    func testTextMatchMaterializationConsumesOnlyOneOptimisticRow() {
        let local = [
            optimisticUser("local-user-1", text: "same text"),
            optimisticUser("local-user-2", text: "same text"),
        ]
        let remote = [historyUser(0, text: "same text")]

        let merged = GaryxTranscriptMerge.mergedMessages(remote, withLocal: local)

        XCTAssertEqual(merged.map(\.id), ["local-user-1", "local-user-2"])
        XCTAssertEqual(merged[0].localState, .remoteFinal)
        XCTAssertEqual(merged[1].localState, .optimistic, "the second send is still in flight")
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
        var failed = optimisticUser("local-user-9", text: "did not make it")
        failed.statusText = "gateway exploded"
        let remote = [historyUser(0, text: "older message")]

        let merged = GaryxTranscriptMerge.mergedMessages(remote, withLocal: [failed])

        XCTAssertEqual(merged.count, 2)
        XCTAssertEqual(merged[1].id, "local-user-9")
        XCTAssertEqual(merged[1].statusText, "gateway exploded")
    }

    func testPendingInputRowConsumesRemoteOccurrence() {
        let pendingRow = GaryxMobileMessage(
            id: "pending-user:p1",
            role: .user,
            text: "do it",
            timestamp: nil,
            isStreaming: false,
            pendingInputId: "p1",
            localState: .remotePartial
        )
        let optimistic = optimisticUser("local-user-2", text: "do it")
        let remote = [historyUser(0, text: "do it")]

        let merged = GaryxTranscriptMerge.mergedMessages(remote, withLocal: [pendingRow, optimistic])

        XCTAssertEqual(merged.count, 2)
        XCTAssertEqual(
            merged[0].id,
            "history:0",
            "the remote occurrence belongs to the pending row, not the optimistic send"
        )
        XCTAssertEqual(merged[1].id, "local-user-2", "optimistic send is preserved")
        XCTAssertEqual(merged[1].localState, .optimistic)
    }

    func testOptimisticSendEchoesImmediatelyAndDedupesWhenCommitted() {
        let optimistic = optimisticUser(
            "local-user-echo",
            text: "continue",
            clientIntentId: "mobile-echo"
        )

        XCTAssertEqual(
            GaryxTranscriptMerge.mergedMessages([], withLocal: [optimistic]).map(\.id),
            ["local-user-echo"]
        )

        let committed = [historyUser(42, text: "continue", clientIntentId: "mobile-echo")]
        let merged = GaryxTranscriptMerge.mergedMessages(committed, withLocal: [optimistic])

        XCTAssertEqual(merged.map(\.id), ["local-user-echo"])
        XCTAssertEqual(merged[0].remoteId, "history:42")
        XCTAssertEqual(merged[0].localState, .remoteFinal)
    }

    func testEmptyRemoteKeepsLocalUntouched() {
        let local = [optimisticUser("local-user-1", text: "hello")]
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
            id: "history:\(index)",
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
