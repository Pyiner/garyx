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

        XCTAssertEqual(merged.map(\.id), ["history:0", "stream-assistant-t1-1"])
        XCTAssertEqual(merged[1].remoteId, "history:1")
        XCTAssertEqual(merged[1].localState, .remoteFinal)
        XCTAssertFalse(merged[1].isStreaming)
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

    func testActiveRunDedupesRunningVsCompletedWhenToolUseIdAbsent() {
        // Same tool call with NO toolUseId (provider/stream omitted it): the live group
        // shows it "running", the committed page shows it "completed". They are one call
        // (same tool + input) and must collapse to ONE row, not show twice — the merge
        // key must not hinge on the volatile summary/status that changes as a call ends.
        func entry(summary: String, status: GaryxMobileToolTraceStatus, result: String?) -> GaryxMobileToolTraceEntry {
            var e = GaryxMobileToolTraceEntry(
                id: "e-\(summary)",
                toolUseId: nil,
                toolName: "Bash",
                title: "Bash",
                inputText: "ls",
                inputLabel: "Call",
                resultLabel: "Result",
                status: status,
                isError: false,
                timestamp: nil,
                primaryPathBadge: nil
            )
            e.summaryText = summary
            e.resultText = result
            return e
        }
        let liveGroup = GaryxMobileToolTraceGroup(
            entries: [entry(summary: "Running command", status: .running, result: nil)],
            live: true
        )
        let live = GaryxMobileMessage(
            id: "tool-group:live",
            role: .tool,
            text: liveGroup.summary,
            timestamp: nil,
            isStreaming: true,
            toolTraceGroup: liveGroup,
            localState: .remotePartial
        )
        let committedGroup = GaryxMobileToolTraceGroup(
            entries: [entry(summary: "Ran command", status: .completed, result: "file.txt")],
            live: false
        )
        let committed = GaryxMobileMessage(
            id: "history:5",
            role: .tool,
            text: committedGroup.summary,
            timestamp: nil,
            isStreaming: false,
            toolTraceGroup: committedGroup,
            localState: .remoteFinal,
            historyIndex: 5
        )

        let merged = GaryxTranscriptMerge.mergedMessages(
            [committed],
            withLocal: [live],
            threadRunActive: true
        )

        XCTAssertEqual(
            merged.count,
            1,
            "running live + completed committed of the same call must not show twice when toolUseId is absent"
        )
    }

    func testActiveRunDedupesWhenLiveLacksToolUseIdButCommittedHasIt() {
        // The codex-style case: the committed row carries the call id, but the live
        // event omitted it. Same call (tool + input) → one row, not two.
        func entry(id: String?, summary: String, status: GaryxMobileToolTraceStatus, result: String?) -> GaryxMobileToolTraceEntry {
            var e = GaryxMobileToolTraceEntry(
                id: "e-\(summary)",
                toolUseId: id,
                toolName: "Bash",
                title: "Bash",
                inputText: "ls -la",
                inputLabel: "Call",
                resultLabel: "Result",
                status: status,
                isError: false,
                timestamp: nil,
                primaryPathBadge: nil
            )
            e.summaryText = summary
            e.resultText = result
            return e
        }
        let liveGroup = GaryxMobileToolTraceGroup(
            entries: [entry(id: nil, summary: "Running command", status: .running, result: nil)],
            live: true
        )
        let live = GaryxMobileMessage(
            id: "tool-group:live",
            role: .tool,
            text: liveGroup.summary,
            timestamp: nil,
            isStreaming: true,
            toolTraceGroup: liveGroup,
            localState: .remotePartial
        )
        let committedGroup = GaryxMobileToolTraceGroup(
            entries: [entry(id: "call_X", summary: "Ran command", status: .completed, result: "ok")],
            live: false
        )
        let committed = GaryxMobileMessage(
            id: "history:7",
            role: .tool,
            text: committedGroup.summary,
            timestamp: nil,
            isStreaming: false,
            toolTraceGroup: committedGroup,
            localState: .remoteFinal,
            historyIndex: 7
        )

        let merged = GaryxTranscriptMerge.mergedMessages([committed], withLocal: [live], threadRunActive: true)
        XCTAssertEqual(merged.count, 1, "a live row missing the id must still match its committed row by tool + input")
    }

    func testRunningLiveCallNamesACommittedResultOnlyRow() {
        // Dual-source skew: the resumable committed stream delivered a tool_RESULT
        // whose tool_use landed before the window opened, so the committed row is a
        // bare result-only entry (generic "tool"). The global stream's call for the
        // same id is still RUNNING and carries the real name + input. The overlay
        // must adopt that identity so the row reads "Ran 1 command" holding the
        // result — never a stray "Used 1 tool" containing the prior result.
        var resultOnly = GaryxMobileToolTraceEntry(
            id: "r", toolUseId: "toolu_KILL", toolName: "tool", title: "Tool",
            inputText: nil, inputLabel: "Input", resultLabel: "Result",
            status: .completed, isError: false, timestamp: nil, primaryPathBadge: nil
        )
        resultOnly.resultText = "done"
        let committedGroup = GaryxMobileToolTraceGroup(entries: [resultOnly], live: false)
        let committed = GaryxMobileMessage(
            id: "history:9", role: .tool, text: committedGroup.summary, timestamp: nil,
            isStreaming: false, toolTraceGroup: committedGroup, localState: .remoteFinal, historyIndex: 9
        )
        let running = GaryxMobileToolTraceEntry(
            id: "u", toolUseId: "toolu_KILL", toolName: "Bash", title: "Bash",
            inputText: "kill %1 2>/dev/null; echo \"done\"", inputLabel: "Call", resultLabel: "Result",
            status: .running, isError: false, timestamp: nil, primaryPathBadge: nil
        )
        let liveGroup = GaryxMobileToolTraceGroup(entries: [running], live: true)
        let live = GaryxMobileMessage(
            id: "tool-group:live", role: .tool, text: liveGroup.summary, timestamp: nil,
            isStreaming: true, toolTraceGroup: liveGroup, localState: .remotePartial
        )

        let merged = GaryxTranscriptMerge.mergedMessages([committed], withLocal: [live], threadRunActive: true)

        let toolRows = merged.filter { $0.role == .tool }
        XCTAssertEqual(toolRows.count, 1, "the running call and its committed result are one row")
        let entry = toolRows.first?.toolTraceGroup?.entries.first
        XCTAssertEqual(entry?.toolName, "Bash", "the result-only row adopts the running call's name")
        XCTAssertEqual(entry?.resultText, "done", "the committed result is preserved")
        XCTAssertEqual(toolRows.first?.text, "Ran 1 command", "not a stray \"Used 1 tool\"")
    }

    func testLiveGroupSpanningCallsCommittedSplitDoesNotDuplicate() {
        // The live builder kept calls A and B in one group (only an assistant
        // boundary, no streamed text, arrived between them), but the committed
        // window has an assistant text row between them and split them into two
        // rows. The overlay routes each live entry to its own committed row, so B
        // is never duplicated into A's row.
        func call(_ id: String, _ input: String, status: GaryxMobileToolTraceStatus, result: String?) -> GaryxMobileToolTraceEntry {
            var e = GaryxMobileToolTraceEntry(
                id: "e-\(id)", toolUseId: id, toolName: "Bash", title: "Bash",
                inputText: input, inputLabel: "Call", resultLabel: "Result",
                status: status, isError: false, timestamp: nil, primaryPathBadge: nil
            )
            e.resultText = result
            return e
        }
        func committedRow(_ id: String, _ input: String, _ result: String, index: Int) -> GaryxMobileMessage {
            let group = GaryxMobileToolTraceGroup(entries: [call(id, input, status: .completed, result: result)], live: false)
            return GaryxMobileMessage(
                id: "history:\(index)", role: .tool, text: group.summary, timestamp: nil,
                isStreaming: false, toolTraceGroup: group, localState: .remoteFinal, historyIndex: index
            )
        }
        let committedA = committedRow("tu-A", "pwd", "/", index: 1)
        let narration = GaryxMobileMessage(
            id: "history:2", role: .assistant, text: "Checking the directory.", timestamp: nil,
            isStreaming: false, localState: .remoteFinal, historyIndex: 2
        )
        let committedB = committedRow("tu-B", "ls", "file", index: 3)
        let liveGroup = GaryxMobileToolTraceGroup(
            entries: [
                call("tu-A", "pwd", status: .running, result: nil),
                call("tu-B", "ls", status: .running, result: nil),
            ],
            live: true
        )
        let live = GaryxMobileMessage(
            id: "tool-group:live", role: .tool, text: liveGroup.summary, timestamp: nil,
            isStreaming: true, toolTraceGroup: liveGroup, localState: .remotePartial
        )

        let merged = GaryxTranscriptMerge.mergedMessages(
            [committedA, narration, committedB], withLocal: [live], threadRunActive: true
        )

        let toolRows = merged.filter { $0.role == .tool }
        XCTAssertEqual(toolRows.count, 2, "A and B keep their own committed rows; neither folds into the other")
        let entries = toolRows.flatMap { $0.toolTraceGroup?.entries ?? [] }
        XCTAssertEqual(entries.filter { $0.toolUseId == "tu-B" }.count, 1, "tu-B renders exactly once")
        XCTAssertEqual(entries.filter { $0.toolUseId == "tu-A" }.count, 1, "tu-A renders exactly once")
    }

    func testRepeatedMergeKeepsInFlightCallStable() {
        // The flush feeds each merged result back as the next local input. A live
        // group of [A (already committed) + B (the latest call, not yet committed)]
        // must reconcile the same way on the second merge — A overlays its committed
        // row, B trails once — so the in-flight row neither jumps position nor
        // duplicates across flushes.
        func call(_ id: String, status: GaryxMobileToolTraceStatus, result: String?) -> GaryxMobileToolTraceEntry {
            var e = GaryxMobileToolTraceEntry(
                id: "e-\(id)", toolUseId: id, toolName: "Bash", title: "Bash",
                inputText: id, inputLabel: "Call", resultLabel: "Result",
                status: status, isError: false, timestamp: nil, primaryPathBadge: nil
            )
            e.resultText = result
            return e
        }
        let user = GaryxMobileMessage(
            id: "history:0", role: .user, text: "go", timestamp: nil,
            isStreaming: false, localState: .remoteFinal, historyIndex: 0
        )
        let committedAGroup = GaryxMobileToolTraceGroup(entries: [call("tu-A", status: .completed, result: "ok")], live: false)
        let committedA = GaryxMobileMessage(
            id: "history:1", role: .tool, text: committedAGroup.summary, timestamp: nil,
            isStreaming: false, toolTraceGroup: committedAGroup, localState: .remoteFinal, historyIndex: 1
        )
        let remote = [user, committedA]
        let liveGroup = GaryxMobileToolTraceGroup(
            entries: [
                call("tu-A", status: .running, result: nil),
                call("tu-B", status: .running, result: nil),
            ],
            live: true
        )
        let live = GaryxMobileMessage(
            id: "tool-group:e-tu-A", role: .tool, text: liveGroup.summary, timestamp: nil,
            isStreaming: true, toolTraceGroup: liveGroup, localState: .remotePartial
        )

        func toolOrder(_ messages: [GaryxMobileMessage]) -> [String] {
            messages.filter { $0.role == .tool }.flatMap { $0.toolTraceGroup?.entries.compactMap(\.toolUseId) ?? [] }
        }
        let merge1 = GaryxTranscriptMerge.mergedMessages(remote, withLocal: [live], threadRunActive: true)
        let merge2 = GaryxTranscriptMerge.mergedMessages(remote, withLocal: merge1, threadRunActive: true)

        XCTAssertEqual(toolOrder(merge1), ["tu-A", "tu-B"], "A overlays its committed row; the in-flight B trails once")
        XCTAssertEqual(toolOrder(merge2), toolOrder(merge1), "re-merging the prior result is stable — no jump, no duplicate")
        XCTAssertEqual(merge2.map(\.id), merge1.map(\.id), "row identities are stable across the feed-back flush")
    }

    func testMidRunSteerDoesNotDuplicateRunningTool() {
        // Mid-run steer: while turn 1 is still running, the user sends a second
        // message M2, appended as an OPTIMISTIC user row at the end of the list.
        // A turn-1 tool tu-2 then streams: its committed row is
        // before M2, its live row lands after M2. They share a stable toolUseId, so
        // they are the same call and must render once — the optimistic M2 between
        // them must not split the turn into a duplicate.
        func bashEntry(_ id: String, status: GaryxMobileToolTraceStatus, result: String?) -> GaryxMobileToolTraceEntry {
            var e = GaryxMobileToolTraceEntry(
                id: "e-\(id)", toolUseId: id, toolName: "Bash", title: "Bash",
                inputText: "echo \(id)", inputLabel: "Call", resultLabel: "Result",
                status: status, isError: false, timestamp: nil, primaryPathBadge: nil
            )
            e.resultText = result
            return e
        }
        let u1 = GaryxMobileMessage(
            id: "history:0", role: .user, text: "first", timestamp: nil,
            isStreaming: false, localState: .remoteFinal, historyIndex: 0
        )
        let committedGroup = GaryxMobileToolTraceGroup(entries: [bashEntry("tu-2", status: .completed, result: "done")], live: false)
        let committedTu2 = GaryxMobileMessage(
            id: "history:1", role: .tool, text: committedGroup.summary, timestamp: nil,
            isStreaming: false, toolTraceGroup: committedGroup, localState: .remoteFinal, historyIndex: 1
        )
        let remote = [u1, committedTu2]

        let m2 = GaryxMobileMessage(
            id: "local-user-M2", role: .user, text: "steer", timestamp: nil,
            isStreaming: false, clientIntentId: "intent-M2", localState: .optimistic
        )
        let liveGroup = GaryxMobileToolTraceGroup(entries: [bashEntry("tu-2", status: .running, result: nil)], live: true)
        let liveTu2 = GaryxMobileMessage(
            id: "tool-group:e-tu-2", role: .tool, text: liveGroup.summary, timestamp: nil,
            isStreaming: true, toolTraceGroup: liveGroup, localState: .remotePartial
        )
        // The local list as the flush sees it: prior merged rows + the optimistic
        // steer + the live tool that streamed after it.
        let local = [u1, committedTu2, m2, liveTu2]

        let merged = GaryxTranscriptMerge.mergedMessages(remote, withLocal: local, threadRunActive: true)

        let tu2Count = merged.flatMap { $0.toolTraceGroup?.entries ?? [] }.filter { $0.toolUseId == "tu-2" }.count
        XCTAssertEqual(tu2Count, 1, "a steer message between the committed and live tu-2 must not duplicate the call")
    }

    func testPendingAckSteerDoesNotDuplicateRunningTool() {
        // Once the gateway acks the queued steer, the mobile client maps the pending
        // input to a user row with .remotePartial (no longer .optimistic). It is
        // still a future turn the assistant has not started, so it must not bound the
        // running turn either — otherwise the turn-1 tu-2 duplicates exactly as in
        // the optimistic case.
        func bashEntry(_ id: String, status: GaryxMobileToolTraceStatus, result: String?) -> GaryxMobileToolTraceEntry {
            var e = GaryxMobileToolTraceEntry(
                id: "e-\(id)", toolUseId: id, toolName: "Bash", title: "Bash",
                inputText: "echo \(id)", inputLabel: "Call", resultLabel: "Result",
                status: status, isError: false, timestamp: nil, primaryPathBadge: nil
            )
            e.resultText = result
            return e
        }
        let u1 = GaryxMobileMessage(
            id: "history:0", role: .user, text: "first", timestamp: nil,
            isStreaming: false, localState: .remoteFinal, historyIndex: 0
        )
        let committedGroup = GaryxMobileToolTraceGroup(entries: [bashEntry("tu-2", status: .completed, result: "done")], live: false)
        let committedTu2 = GaryxMobileMessage(
            id: "history:1", role: .tool, text: committedGroup.summary, timestamp: nil,
            isStreaming: false, toolTraceGroup: committedGroup, localState: .remoteFinal, historyIndex: 1
        )
        // The acked steer: a pending input mapped to a .remotePartial user row.
        let pendingM2 = GaryxMobileMessage(
            id: "queued:M2", role: .user, text: "steer", timestamp: nil,
            isStreaming: false, pendingInputId: "queued_input:1", localState: .remotePartial
        )
        let remote = [u1, committedTu2, pendingM2]
        let liveGroup = GaryxMobileToolTraceGroup(entries: [bashEntry("tu-2", status: .running, result: nil)], live: true)
        let liveTu2 = GaryxMobileMessage(
            id: "tool-group:e-tu-2", role: .tool, text: liveGroup.summary, timestamp: nil,
            isStreaming: true, toolTraceGroup: liveGroup, localState: .remotePartial
        )

        let merged = GaryxTranscriptMerge.mergedMessages(remote, withLocal: [liveTu2], threadRunActive: true)

        let tu2Count = merged.flatMap { $0.toolTraceGroup?.entries ?? [] }.filter { $0.toolUseId == "tu-2" }.count
        XCTAssertEqual(tu2Count, 1, "an acked (.remotePartial) steer must not bound the turn and duplicate tu-2")
    }

    func testMaterializedMidRunUserDropsStaleStreamingAssistantFromPreviousTurn() {
        let firstAssistantText = """
        I already explained the active-run merge path and the prior live assistant row.
        """
        let remote = [
            historyUser(0, text: "first"),
            historyAssistant(1, text: firstAssistantText),
            historyUser(2, text: "follow-up", clientIntentId: "mobile-follow-up"),
            historyAssistant(3, text: "new answer"),
        ]
        let localFollowUp = optimisticUser(
            "local-user-follow-up",
            text: "follow-up",
            clientIntentId: "mobile-follow-up"
        )
        let staleStreamingAssistant = GaryxMobileMessage(
            id: "stream-assistant-old",
            role: .assistant,
            text: firstAssistantText,
            timestamp: nil,
            isStreaming: true,
            localState: .remotePartial
        )
        let local = [
            remote[0],
            remote[1],
            localFollowUp,
            staleStreamingAssistant,
        ]

        let merged = GaryxTranscriptMerge.mergedMessages(remote, withLocal: local, threadRunActive: true)

        XCTAssertEqual(merged.map(\.id), [
            "history:0",
            "history:1",
            "local-user-follow-up",
            "history:3",
        ])
        XCTAssertEqual(
            merged.filter { $0.role == .assistant && $0.text == firstAssistantText }.count,
            1,
            "the previous turn's streaming assistant copy must not be pinned below the follow-up"
        )
    }

    func testCodexTrailingAssistantCopyKeepsLiveIdentityWhenCommitted() throws {
        let finalAnswer = """
        The deployment target should be selected from the preview environment. \
        Use a synthetic host in tests and keep production credentials out of fixtures.
        """
        let committed = GaryxMobileTranscriptMapper.mobileMessages(from: [
            GaryxTranscriptMessage(
                index: 664,
                role: .user,
                text: "Why was this validated in environment A instead of environment B?"
            ),
            GaryxTranscriptMessage(
                index: 665,
                role: .toolUse,
                content: json(#"{"type":"reasoning","id":"rs_synthetic","content":[],"summary":[]}"#)
            ),
            GaryxTranscriptMessage(
                index: 666,
                role: .toolResult,
                content: json(#"{"type":"reasoning","id":"rs_synthetic","content":[],"summary":[]}"#)
            ),
            GaryxTranscriptMessage(index: 667, role: .assistant, text: finalAnswer),
        ])
        XCTAssertEqual(
            committed.filter { $0.role == .assistant && $0.text == finalAnswer }.count,
            1,
            "the committed-only mapper should render the Codex final answer once"
        )
        XCTAssertTrue(committed.filter { $0.role == .tool }.isEmpty, "reasoning records stay hidden")

        let liveUser = optimisticUser(
            "local-user-codex",
            text: "Why was this validated in environment A instead of environment B?"
        )
        let liveAssistant = GaryxMobileMessage(
            id: "local-assistant-codex",
            role: .assistant,
            text: finalAnswer,
            timestamp: nil,
            isStreaming: true,
            localState: .remotePartial
        )

        var merged = GaryxTranscriptMerge.mergedMessages(
            committed,
            withLocal: [liveUser, liveAssistant],
            threadRunActive: true
        )

        if !merged.contains(where: { $0.id == liveAssistant.id }) {
            // Mirrors GaryxMobileModel+Streaming: once the committed reload loses
            // the active assistant row id, a late live copy is appended as a new
            // remote_partial assistant row instead of reconciling with history:667.
            merged.append(liveAssistant)
        }

        let rows = GaryxMobileTurnRenderer.buildTurnRows(messages: merged, isRunningThread: false)
        XCTAssertEqual(renderedTextCount(finalAnswer, in: rows), 1)
    }

    func testCodexInterToolTextSurvivesDelayedLiveFlushAfterCommittedMerge() throws {
        let interToolText = "The first batch finished; I am checking the remaining generated files."
        let committed = codexInterToolCommittedMessages(interToolText: interToolText)
        let committedSignature = renderedActivitySignature(messages: committed, isRunningThread: true)
        XCTAssertEqual(
            committedSignature,
            [
                "tool:Ran 3 commands",
                "assistant:\(interToolText)",
                "tool:Ran 2 commands",
            ],
            "pure committed rendering is the canonical group/text/group order"
        )

        var liveAdjacentToolState = Array(committed.prefix(2))
        liveAdjacentToolState.append(
            GaryxMobileMessage(
                id: "stream-assistant-placeholder",
                role: .assistant,
                text: "",
                timestamp: nil,
                isStreaming: true,
                localState: .remotePartial
            )
        )
        GaryxTranscriptMerge.appendLiveToolTraceEntry(
            toolTraceEntry(id: "call-4", status: .running),
            kind: .toolUse,
            into: &liveAdjacentToolState
        )
        GaryxTranscriptMerge.appendLiveToolTraceEntry(
            toolTraceEntry(id: "call-5", status: .running),
            kind: .toolUse,
            into: &liveAdjacentToolState
        )
        XCTAssertEqual(
            renderedActivitySignature(messages: liveAdjacentToolState, isRunningThread: true),
            [
                "tool:Ran 3 commands",
                "tool:Ran 2 commands",
            ],
            "the live-only state reproduces the adjacent tool groups: the assistant delta is still buffered"
        )

        var merged = GaryxTranscriptMerge.mergedMessages(
            committed,
            withLocal: liveAdjacentToolState,
            threadRunActive: true
        )
        XCTAssertEqual(
            renderedActivitySignature(messages: merged, isRunningThread: true),
            committedSignature,
            "committed rows restore the group/text/group structure before the delayed assistant delta flushes"
        )

        GaryxTranscriptMerge.appendLiveAssistantText(
            interToolText,
            targetId: "stream-assistant-inter-tool",
            into: &merged
        )

        XCTAssertEqual(renderedActivitySignature(messages: merged, isRunningThread: true), committedSignature)
        XCTAssertEqual(renderedTextCount(interToolText, in: GaryxMobileTurnRenderer.buildTurnRows(messages: merged, isRunningThread: true)), 1)
    }

    func testCodexFinalAssistantTextSurvivesDelayedLiveFlushAfterCommittedMerge() throws {
        let finalAnswer = """
        The preview lane has been validated with synthetic data; keep real user identifiers out of fixtures.
        """
        let committed = GaryxMobileTranscriptMapper.mobileMessages(from: [
            GaryxTranscriptMessage(index: 664, role: .user, text: "Summarize the validation."),
            GaryxTranscriptMessage(
                index: 665,
                role: .toolUse,
                content: json(#"{"type":"reasoning","id":"rs_synthetic_final","content":[],"summary":[]}"#)
            ),
            GaryxTranscriptMessage(
                index: 666,
                role: .toolResult,
                content: json(#"{"type":"reasoning","id":"rs_synthetic_final","content":[],"summary":[]}"#)
            ),
            GaryxTranscriptMessage(index: 667, role: .assistant, text: finalAnswer),
        ])
        var merged = GaryxTranscriptMerge.mergedMessages(
            committed,
            withLocal: [optimisticUser("local-user-final", text: "Summarize the validation.")],
            threadRunActive: true
        )

        GaryxTranscriptMerge.appendLiveAssistantText(
            finalAnswer,
            targetId: "stream-assistant-final",
            into: &merged
        )

        let rows = GaryxMobileTurnRenderer.buildTurnRows(messages: merged, isRunningThread: false)
        XCTAssertEqual(renderedTextCount(finalAnswer, in: rows), 1)
    }

    func testDistinctIdToolsAcrossSteerDoNotFold() {
        // A steer makes the boundary transparent for STABLE-ID reconciliation, but a
        // genuinely new turn-2 call has its own unique toolUseId (ids are unique per
        // call), so it must NOT fold into a same-command turn-1 committed row across
        // the steer — it gets its own row. Guards the id-reconciliation from
        // over-merging two distinct calls.
        func bashEntry(_ id: String, status: GaryxMobileToolTraceStatus, result: String?) -> GaryxMobileToolTraceEntry {
            var e = GaryxMobileToolTraceEntry(
                id: "e-\(id)", toolUseId: id, toolName: "Bash", title: "Bash",
                inputText: "ls", inputLabel: "Call", resultLabel: "Result",
                status: status, isError: false, timestamp: nil, primaryPathBadge: nil
            )
            e.resultText = result
            return e
        }
        let u1 = GaryxMobileMessage(
            id: "history:0", role: .user, text: "first", timestamp: nil,
            isStreaming: false, localState: .remoteFinal, historyIndex: 0
        )
        // turn-1 ran `ls` as tu-A (committed).
        let committedGroup = GaryxMobileToolTraceGroup(entries: [bashEntry("tu-A", status: .completed, result: "done")], live: false)
        let committedA = GaryxMobileMessage(
            id: "history:1", role: .tool, text: committedGroup.summary, timestamp: nil,
            isStreaming: false, toolTraceGroup: committedGroup, localState: .remoteFinal, historyIndex: 1
        )
        let pendingM2 = GaryxMobileMessage(
            id: "queued:M2", role: .user, text: "steer", timestamp: nil,
            isStreaming: false, pendingInputId: "queued_input:1", localState: .remotePartial
        )
        let remote = [u1, committedA, pendingM2]
        // turn-2 runs `ls` again, but as a DISTINCT call tu-B (new unique id).
        let liveGroup = GaryxMobileToolTraceGroup(entries: [bashEntry("tu-B", status: .running, result: nil)], live: true)
        let liveB = GaryxMobileMessage(
            id: "tool-group:e-tu-B", role: .tool, text: liveGroup.summary, timestamp: nil,
            isStreaming: true, toolTraceGroup: liveGroup, localState: .remotePartial
        )

        let merged = GaryxTranscriptMerge.mergedMessages(remote, withLocal: [liveB], threadRunActive: true)

        let ids = merged.flatMap { $0.toolTraceGroup?.entries.compactMap(\.toolUseId) ?? [] }
        XCTAssertEqual(ids.filter { $0 == "tu-A" }.count, 1, "turn-1 tu-A stays its own row")
        XCTAssertEqual(ids.filter { $0 == "tu-B" }.count, 1, "the new turn-2 tu-B does not fold into tu-A")
    }

    func testToolTraceEntriesSameCallSemantics() {
        func e(_ id: String?, input: String = "ls") -> GaryxMobileToolTraceEntry {
            GaryxMobileToolTraceEntry(
                id: id ?? "x",
                toolUseId: id,
                toolName: "Bash",
                title: "Bash",
                inputText: input,
                inputLabel: "Call",
                resultLabel: "Result",
                status: .running,
                isError: false,
                timestamp: nil,
                primaryPathBadge: nil
            )
        }
        // Both identified → match only on equal id.
        XCTAssertTrue(GaryxTranscriptMerge.toolTraceEntriesSameCall(e("call_A"), e("call_A"), allowFingerprint: true))
        XCTAssertFalse(
            GaryxTranscriptMerge.toolTraceEntriesSameCall(e("call_A"), e("call_B"), allowFingerprint: true),
            "distinct ids are distinct calls even with identical input"
        )
        // At least one unidentified → match on tool + input.
        XCTAssertTrue(
            GaryxTranscriptMerge.toolTraceEntriesSameCall(e("call_A"), e(nil), allowFingerprint: true),
            "id + no-id of the same tool+input is the same call"
        )
        XCTAssertFalse(
            GaryxTranscriptMerge.toolTraceEntriesSameCall(e("call_A"), e(nil, input: "pwd"), allowFingerprint: true),
            "different input is a different call"
        )
        // Fingerprint disabled (older turn) → no fingerprint fallback.
        XCTAssertFalse(GaryxTranscriptMerge.toolTraceEntriesSameCall(e(nil), e(nil), allowFingerprint: false))
    }

    // MARK: absorbToolResult (write-time idempotency — the phantom fix)

    private func toolTraceEntry(
        id: String?,
        toolName: String = "Bash",
        input: String? = "ls",
        status: GaryxMobileToolTraceStatus = .running,
        result: String? = nil
    ) -> GaryxMobileToolTraceEntry {
        var e = GaryxMobileToolTraceEntry(
            id: id ?? "entry",
            toolUseId: id,
            toolName: toolName,
            title: toolName,
            inputText: input,
            inputLabel: "Call",
            resultLabel: "Result",
            status: status,
            isError: false,
            timestamp: nil,
            primaryPathBadge: nil
        )
        e.resultText = result
        return e
    }

    func testAbsorbResultConsumesDuplicateIdResultIdempotently() {
        // The phantom case: the call ALREADY has its result (its committed copy
        // raced ahead of the live event under the real-time stream). A late result
        // with the same id must be absorbed in place — never spawn a second row.
        var group = GaryxMobileToolTraceGroup(
            entries: [toolTraceEntry(id: "tu-1", status: .completed, result: "done")],
            live: false
        )
        let lateResult = toolTraceEntry(id: "tu-1", toolName: "tool", input: nil, status: .completed, result: "done")
        XCTAssertTrue(
            GaryxTranscriptMerge.absorbToolResult(lateResult, into: &group),
            "a same-id result must be absorbed even when the call already has a result"
        )
        XCTAssertEqual(group.entries.count, 1, "no phantom result-only entry is created")
        // Idempotent: absorbing again changes nothing.
        XCTAssertTrue(GaryxTranscriptMerge.absorbToolResult(lateResult, into: &group))
        XCTAssertEqual(group.entries.count, 1)
        XCTAssertEqual(group.entries[0].resultText, "done")
        XCTAssertEqual(group.entries[0].toolName, "Bash", "a generic late result must not clobber the real tool name")
    }

    func testAbsorbResultCompletesOpenCall() {
        var group = GaryxMobileToolTraceGroup(entries: [toolTraceEntry(id: "tu-1", status: .running)], live: true)
        XCTAssertTrue(
            GaryxTranscriptMerge.absorbToolResult(
                toolTraceEntry(id: "tu-1", status: .completed, result: "file.txt"),
                into: &group
            )
        )
        XCTAssertEqual(group.entries.count, 1)
        XCTAssertEqual(group.entries[0].status, .completed)
        XCTAssertEqual(group.entries[0].resultText, "file.txt")
    }

    func testAbsorbResultReturnsFalseWhenNoMatchingCall() {
        // An id'd result whose call is not in this group: the live append path must
        // DROP it, never render a lone result row.
        var group = GaryxMobileToolTraceGroup(entries: [toolTraceEntry(id: "tu-1", status: .running)], live: true)
        let orphan = toolTraceEntry(id: "tu-OTHER", toolName: "tool", input: nil, status: .completed, result: "x")
        XCTAssertFalse(GaryxTranscriptMerge.absorbToolResult(orphan, into: &group))
        XCTAssertEqual(group.entries.count, 1, "an unmatched result is never inserted as an entry")
    }

    func testAbsorbResultIdlessFallbackMatchesRunningEntry() {
        // Gemini unkeyed: no ids on either side. The result matches the OPEN running
        // entry by tool name (the required fingerprint/running fallback is kept).
        var group = GaryxMobileToolTraceGroup(entries: [toolTraceEntry(id: nil, status: .running)], live: true)
        XCTAssertTrue(
            GaryxTranscriptMerge.absorbToolResult(
                toolTraceEntry(id: nil, input: nil, status: .completed, result: "ok"),
                into: &group
            )
        )
        XCTAssertEqual(group.entries.count, 1)
        XCTAssertEqual(group.entries[0].resultText, "ok")
    }

    func testAbsorbResultIdlessReturnsFalseWithoutRunningEntry() {
        // No id and no OPEN call to attach to → drop (committed is authoritative);
        // a finished id-less call is never re-opened by a stray result.
        var group = GaryxMobileToolTraceGroup(
            entries: [toolTraceEntry(id: nil, status: .completed, result: "done")],
            live: false
        )
        XCTAssertFalse(
            GaryxTranscriptMerge.absorbToolResult(
                toolTraceEntry(id: nil, input: nil, status: .completed, result: "again"),
                into: &group
            )
        )
        XCTAssertEqual(group.entries.count, 1)
    }

    func testLiveToolUseForAlreadyShownCallDoesNotDuplicate() {
        // Dual-source transient: the committed copy of a command is already shown
        // as a completed "Ran 1 command" (followed by assistant text). A late live
        // tool_use + tool_result for the SAME call (normalized to a generic
        // payload) must NOT open a second "Used 1 tool" group.
        let committed = GaryxMobileToolTraceGroup(
            entries: [toolTraceEntry(id: "tu-1", status: .completed, result: "done")],
            live: false
        )
        var messages: [GaryxMobileMessage] = [
            GaryxMobileMessage(
                id: "tool-group:c", role: .tool, text: committed.summary, timestamp: nil,
                isStreaming: false, toolTraceGroup: committed, localState: .remoteFinal
            ),
            GaryxMobileMessage(
                id: "a1", role: .assistant, text: "running…", timestamp: nil,
                isStreaming: false, localState: .remoteFinal
            ),
        ]
        GaryxTranscriptMerge.appendLiveToolTraceEntry(
            toolTraceEntry(id: "tu-1", toolName: "tool", input: nil, status: .running),
            kind: .toolUse, into: &messages
        )
        XCTAssertEqual(
            messages.filter { $0.role == .tool }.count, 1,
            "a duplicate live tool_use for an already-shown call must not open a second group"
        )
        GaryxTranscriptMerge.appendLiveToolTraceEntry(
            toolTraceEntry(id: "tu-1", toolName: "tool", input: nil, status: .completed, result: "done"),
            kind: .toolResult, into: &messages
        )
        let toolRows = messages.filter { $0.role == .tool }
        XCTAssertEqual(toolRows.count, 1, "still exactly one tool row after the late result")
        XCTAssertEqual(toolRows.first?.toolTraceGroup?.summary, "Ran 1 command")
    }

    func testLiveToolUseForNewCallOpensAGroup() {
        var messages: [GaryxMobileMessage] = [
            GaryxMobileMessage(id: "a1", role: .assistant, text: "hi", timestamp: nil, isStreaming: false, localState: .remoteFinal),
        ]
        GaryxTranscriptMerge.appendLiveToolTraceEntry(
            toolTraceEntry(id: "tu-9", toolName: "Bash", input: "ls", status: .running),
            kind: .toolUse, into: &messages
        )
        XCTAssertEqual(messages.filter { $0.role == .tool }.count, 1, "a genuinely new call opens a group")
    }

    func testStraddlingResultAbsorbsIntoFlushedGroupWithinTurn() {
        // A tool_use was flushed into an earlier group (a parent text row split it
        // from its result while a sub-agent ran). The straddling result must
        // absorb into that group, not render as a standalone "Used 1 tool".
        let flushed = GaryxMobileToolTraceGroup(entries: [toolTraceEntry(id: "tu-1", status: .running)], live: false)
        var messages: [GaryxMobileMessage] = [
            GaryxMobileMessage(id: "tool:1", role: .tool, text: flushed.summary, timestamp: nil,
                               isStreaming: false, toolTraceGroup: flushed, localState: .remoteFinal),
            GaryxMobileMessage(id: "a1", role: .assistant, text: "narrating…", timestamp: nil,
                               isStreaming: false, localState: .remoteFinal),
        ]
        XCTAssertTrue(
            GaryxTranscriptMerge.absorbResultIntoFlushedToolGroup(
                toolTraceEntry(id: "tu-1", status: .completed, result: "done"), in: &messages
            )
        )
        XCTAssertEqual(messages.filter { $0.role == .tool }.count, 1)
        XCTAssertEqual(messages.first?.toolTraceGroup?.entries.first?.status, .completed)
        XCTAssertEqual(messages.first?.toolTraceGroup?.entries.first?.resultText, "done")
    }

    func testStraddlingResultDoesNotCrossUserTurnBoundary() {
        let prior = GaryxMobileToolTraceGroup(entries: [toolTraceEntry(id: "tu-1", status: .running)], live: false)
        var messages: [GaryxMobileMessage] = [
            GaryxMobileMessage(id: "tool:prior", role: .tool, text: prior.summary, timestamp: nil,
                               isStreaming: false, toolTraceGroup: prior, localState: .remoteFinal),
            GaryxMobileMessage(id: "u1", role: .user, text: "next question", timestamp: nil,
                               isStreaming: false, localState: .remoteFinal),
        ]
        XCTAssertFalse(
            GaryxTranscriptMerge.absorbResultIntoFlushedToolGroup(
                toolTraceEntry(id: "tu-1", status: .completed, result: "late"), in: &messages
            ),
            "a result in a new turn must not reach back across the user row"
        )
    }

    func testStraddlingResultRequiresStableIdAcrossGroups() {
        // an id-less result must not attach to an unrelated generic group by the
        // weak tool-name fallback when crossing flushed groups.
        let group = GaryxMobileToolTraceGroup(entries: [toolTraceEntry(id: nil, status: .running)], live: false)
        var messages: [GaryxMobileMessage] = [
            GaryxMobileMessage(id: "tool:1", role: .tool, text: group.summary, timestamp: nil,
                               isStreaming: false, toolTraceGroup: group, localState: .remoteFinal),
            GaryxMobileMessage(id: "a1", role: .assistant, text: "x", timestamp: nil,
                               isStreaming: false, localState: .remoteFinal),
        ]
        XCTAssertFalse(
            GaryxTranscriptMerge.absorbResultIntoFlushedToolGroup(
                toolTraceEntry(id: nil, toolName: "tool", input: nil, status: .completed, result: "y"), in: &messages
            )
        )
    }

    func testLiveGroupMatchesCommittedOnlyWithinCurrentTurn() {
        func toolMessage(_ id: String, msgId: String, history: Int?, live: Bool) -> GaryxMobileMessage {
            let group = GaryxMobileToolTraceGroup(
                entries: [toolTraceEntry(id: id, status: live ? .running : .completed, result: live ? nil : "ok")],
                live: live
            )
            return GaryxMobileMessage(
                id: msgId, role: .tool, text: group.summary, timestamp: nil,
                isStreaming: live, toolTraceGroup: group,
                localState: live ? .remotePartial : .remoteFinal, historyIndex: history
            )
        }
        func userRow(_ msgId: String, history: Int) -> GaryxMobileMessage {
            GaryxMobileMessage(
                id: msgId, role: .user, text: "q", timestamp: nil,
                isStreaming: false, localState: .remoteFinal, historyIndex: history
            )
        }

        // Cross-turn id reuse: committed tu-1 sits in a PRIOR turn (before the last
        // user); the live tu-1 is a NEW call this turn → it must NOT fold into the
        // old committed row.
        let crossTurn = GaryxTranscriptMerge.mergedMessages(
            [toolMessage("tu-1", msgId: "history:1", history: 1, live: false), userRow("history:2", history: 2)],
            withLocal: [toolMessage("tu-1", msgId: "tool-group:live", history: nil, live: true)],
            threadRunActive: true
        )
        XCTAssertEqual(crossTurn.count, 3, "a reused id in a new turn must not merge into the prior turn's committed row")

        // Same-turn: committed tu-2 is after the user → the live tu-2 collapses into it.
        let sameTurn = GaryxTranscriptMerge.mergedMessages(
            [userRow("history:3", history: 3), toolMessage("tu-2", msgId: "history:4", history: 4, live: false)],
            withLocal: [toolMessage("tu-2", msgId: "tool-group:live2", history: nil, live: true)],
            threadRunActive: true
        )
        XCTAssertEqual(sameTurn.count, 2, "same-turn live + committed of one call collapse to one row")
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

    private func codexInterToolCommittedMessages(interToolText: String) -> [GaryxMobileMessage] {
        var transcript: [GaryxTranscriptMessage] = [
            GaryxTranscriptMessage(index: 100, role: .user, text: "Run the synthetic validation."),
        ]
        for offset in 1...3 {
            transcript.append(commandTranscript(index: 100 + offset * 2 - 1, toolUseId: "call-\(offset)", command: "cmd-\(offset)"))
            transcript.append(commandResultTranscript(index: 100 + offset * 2, toolUseId: "call-\(offset)"))
        }
        transcript.append(
            GaryxTranscriptMessage(
                index: 107,
                role: .toolUse,
                content: json(#"{"type":"reasoning","id":"rs_synthetic_inter_tool","content":[],"summary":[]}"#)
            )
        )
        transcript.append(
            GaryxTranscriptMessage(
                index: 108,
                role: .toolResult,
                content: json(#"{"type":"reasoning","id":"rs_synthetic_inter_tool","content":[],"summary":[]}"#)
            )
        )
        transcript.append(GaryxTranscriptMessage(index: 109, role: .assistant, text: interToolText))
        for offset in 4...5 {
            transcript.append(commandTranscript(index: 110 + (offset - 4) * 2, toolUseId: "call-\(offset)", command: "cmd-\(offset)"))
            transcript.append(commandResultTranscript(index: 111 + (offset - 4) * 2, toolUseId: "call-\(offset)"))
        }
        return GaryxMobileTranscriptMapper.mobileMessages(from: transcript)
    }

    private func commandTranscript(index: Int, toolUseId: String, command: String) -> GaryxTranscriptMessage {
        GaryxTranscriptMessage(
            index: index,
            role: .toolUse,
            content: json(#"{"tool":"Bash","input":{"command":"\#(command)"}}"#),
            toolUseId: toolUseId
        )
    }

    private func commandResultTranscript(index: Int, toolUseId: String) -> GaryxTranscriptMessage {
        GaryxTranscriptMessage(
            index: index,
            role: .toolResult,
            content: json(#"{"result":{"stdout":"ok"}}"#),
            toolUseId: toolUseId
        )
    }

    private func renderedActivitySignature(
        messages: [GaryxMobileMessage],
        isRunningThread: Bool
    ) -> [String] {
        let rows = GaryxMobileTurnRenderer.buildTurnRows(
            messages: messages,
            isRunningThread: isRunningThread
        )
        return rows.flatMap { row in
            row.activityRows.flatMap { activity -> [String] in
                switch activity {
                case .flat(let block):
                    return [renderedSignature(for: block)]
                case .turn(let turn):
                    let steps = turn.steps.map(renderedSignature(for:))
                    if let finalBlock = turn.finalBlock {
                        return steps + [renderedSignature(for: finalBlock)]
                    }
                    return steps
                }
            }
        }
    }

    private func renderedSignature(for block: GaryxMobileTranscriptBlock) -> String {
        switch block {
        case .toolGroup(let message):
            return "tool:\(message.text)"
        case .message(let message):
            let role: String
            switch message.role {
            case .user:
                role = "user"
            case .assistant:
                role = "assistant"
            case .system:
                role = "system"
            case .tool:
                role = "tool"
            }
            return "\(role):\(message.text)"
        }
    }

    private func renderedTextCount(_ text: String, in rows: [GaryxMobileTurnRow]) -> Int {
        rows.reduce(0) { count, row in
            count + row.activityRows.reduce(0) { activityCount, activityRow in
                switch activityRow {
                case .flat(let block):
                    return activityCount + (block.message.text == text ? 1 : 0)
                case .turn(let turn):
                    let stepCount = turn.steps.filter { $0.message.text == text }.count
                    let finalCount = turn.finalBlock?.message.text == text ? 1 : 0
                    return activityCount + stepCount + finalCount
                }
            }
        }
    }

    private func json(_ raw: String) -> GaryxJSONValue {
        try! JSONDecoder().decode(GaryxJSONValue.self, from: Data(raw.utf8))
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
