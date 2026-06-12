import XCTest
@testable import GaryxMobileCore

final class GaryxTranscriptMergeTests: XCTestCase {
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

    func testStreamingAssistantAheadOfRemoteKeepsLocalContent() {
        let local = [
            GaryxMobileMessage(
                id: "history:1",
                role: .assistant,
                text: "partial answer plus newer streamed text",
                timestamp: nil,
                isStreaming: true,
                localState: .remotePartial,
                historyIndex: 1
            ),
        ]
        let remote = [historyAssistant(1, text: "partial answer")]

        let merged = GaryxTranscriptMerge.mergedMessages(remote, withLocal: local)

        XCTAssertEqual(merged.count, 1)
        XCTAssertEqual(merged[0].text, "partial answer plus newer streamed text")
        XCTAssertTrue(merged[0].isStreaming)
    }

    func testStreamingPlaceholderDropsWhenRemoteAlreadyMaterializedTurn() {
        let local = [
            historyUser(0, text: "question"),
            GaryxMobileMessage(
                id: "stream-assistant-t1-1",
                role: .assistant,
                text: "final answer",
                timestamp: nil,
                isStreaming: true,
                localState: .remotePartial
            ),
        ]
        let remote = [
            historyUser(0, text: "question"),
            historyAssistant(1, text: "final answer with more detail"),
        ]

        let merged = GaryxTranscriptMerge.mergedMessages(remote, withLocal: local)

        XCTAssertEqual(merged.map(\.id), ["history:0", "history:1"])
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
        // The remote transcript materialized a queued input; the pending-user
        // row (remote_partial) must consume that occurrence so an unrelated
        // optimistic send with the same text still renders.
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

    func testLiveToolGroupMergesIntoRemoteRow() {
        let runningEntry = GaryxMobileToolTraceEntry(
            id: "e1",
            toolUseId: "tu-1",
            toolName: "Bash",
            title: "Bash",
            inputText: "ls",
            inputLabel: "Input",
            resultLabel: "Result",
            status: .running,
            isError: false,
            timestamp: nil,
            primaryPathBadge: nil
        )
        var completedEntry = runningEntry
        completedEntry.status = .completed
        completedEntry.resultText = "file.txt"

        let localGroup = GaryxMobileToolTraceGroup(entries: [runningEntry], live: true)
        let localTool = GaryxMobileMessage(
            id: "tool-group:e1",
            role: .tool,
            text: localGroup.summary,
            timestamp: nil,
            isStreaming: true,
            toolTraceGroup: localGroup,
            localState: .remotePartial
        )
        let remoteGroup = GaryxMobileToolTraceGroup(entries: [completedEntry], live: false)
        let remoteTool = GaryxMobileMessage(
            id: "tool-group:remote-e1",
            role: .tool,
            text: remoteGroup.summary,
            timestamp: nil,
            isStreaming: false,
            toolTraceGroup: remoteGroup,
            localState: .remoteFinal
        )

        let merged = GaryxTranscriptMerge.mergedMessages([remoteTool], withLocal: [localTool])

        XCTAssertEqual(merged.count, 1)
        XCTAssertEqual(merged[0].id, "tool-group:remote-e1")
        XCTAssertEqual(merged[0].toolTraceGroup?.entries.count, 1)
        XCTAssertEqual(merged[0].toolTraceGroup?.entries[0].resultText, "file.txt")
    }

    func testEmptyRemoteKeepsLocalUntouched() {
        let local = [optimisticUser("local-user-1", text: "hello")]
        XCTAssertEqual(GaryxTranscriptMerge.mergedMessages([], withLocal: local), local)
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

    private func localLiveToolGroup(_ id: String, toolUseId: String) -> GaryxMobileMessage {
        let group = GaryxMobileToolTraceGroup(
            entries: [commandEntry("\(id)-e1", toolUseId: toolUseId, status: .running)],
            live: true
        )
        return GaryxMobileMessage(
            id: "tool-group:\(id)",
            role: .tool,
            text: group.summary,
            timestamp: nil,
            isStreaming: true,
            toolTraceGroup: group,
            localState: .remotePartial
        )
    }

    func testCompletedRunDropsStaleLiveToolGroupInsteadOfTrailingFinalAnswer() {
        // A local tool group that lost its terminal events (backgrounded
        // stream) stays "running" forever. Its rows can also be outside the
        // fetched page of a long thread, so no remote group overlaps it. Once
        // the run is inactive the canonical page wins: the group must be
        // dropped, not appended after the final assistant answer.
        let local = [
            historyUser(0, text: "question"),
            localLiveToolGroup("stale", toolUseId: "call-stale"),
        ]
        let remote = [
            historyUser(0, text: "question"),
            historyAssistant(60, text: "final answer"),
        ]

        let merged = GaryxTranscriptMerge.mergedMessages(
            remote,
            withLocal: local,
            threadRunActive: false
        )

        XCTAssertEqual(merged.map(\.id), ["history:0", "history:60"])
        XCTAssertEqual(merged.last?.role, .assistant, "the final answer stays the last row")
    }

    func testActiveRunStillKeepsUnmatchedLiveToolGroup() {
        let local = [
            historyUser(0, text: "question"),
            localLiveToolGroup("inflight", toolUseId: "call-inflight"),
        ]
        let remote = [
            historyUser(0, text: "question"),
            historyAssistant(1, text: "intermediate segment"),
        ]

        let merged = GaryxTranscriptMerge.mergedMessages(
            remote,
            withLocal: local,
            threadRunActive: true
        )

        XCTAssertEqual(
            merged.map(\.id),
            ["history:0", "history:1", "tool-group:inflight"],
            "in-flight activity not yet persisted must stay visible mid-run"
        )
    }

    func testPreserveRemoteBeforeIndexKeepsOlderToolGroups() {
        // Remote-mapped tool groups carry the first grouped row's transcript
        // index, so older loaded pages keep their tool activity across
        // reconciles just like their text rows.
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
            preserveRemoteBeforeIndex: 40,
            threadRunActive: false
        )

        XCTAssertEqual(
            merged.map(\.id),
            ["history:0", "tool-group:old", "history:2", "history:40", "history:41"]
        )
    }
}
