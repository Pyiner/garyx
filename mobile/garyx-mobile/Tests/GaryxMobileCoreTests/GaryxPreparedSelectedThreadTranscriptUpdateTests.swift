import XCTest
@testable import GaryxMobileCore

final class GaryxPreparedSelectedThreadTranscriptUpdateTests: XCTestCase {
    func testThreadMessagesMarksTrackedAssistantStreamingOnlyWhenBusy() {
        let assistant = mobileMessage("assistant-1", role: .assistant, text: "working")

        let busy = GaryxPreparedThreadMessages.make(
            messages: [assistant],
            isThreadBusy: true,
            activeAssistantMessageId: "assistant-1"
        )

        XCTAssertEqual(busy.activeAssistantMessageId, "assistant-1")
        XCTAssertTrue(busy.messages[0].isStreaming)

        let idle = GaryxPreparedThreadMessages.make(
            messages: [assistant],
            isThreadBusy: false,
            activeAssistantMessageId: "assistant-1"
        )

        XCTAssertNil(idle.activeAssistantMessageId)
        XCTAssertFalse(idle.messages[0].isStreaming)
    }

    func testThreadMessagesAdoptsLastStreamingAssistantWhenBusy() {
        var firstAssistant = mobileMessage("assistant-1", role: .assistant, text: "first")
        firstAssistant.isStreaming = true
        var secondAssistant = mobileMessage("assistant-2", role: .assistant, text: "second")
        secondAssistant.isStreaming = true

        let prepared = GaryxPreparedThreadMessages.make(
            messages: [firstAssistant, secondAssistant],
            isThreadBusy: true,
            activeAssistantMessageId: nil
        )

        XCTAssertEqual(prepared.activeAssistantMessageId, "assistant-2")
        XCTAssertTrue(prepared.messages[1].isStreaming)
    }

    func testThreadMessagesMergesRemoteWindowWithPreservedLocalState() {
        let remote = [
            historyUser(40, text: "new question"),
            historyAssistant(41, text: "new answer"),
        ]
        let local = [
            historyUser(0, text: "old question"),
            optimisticUser("origin:00000000-0000-0000-0000-000000000001", text: "queued follow-up"),
        ]

        let prepared = GaryxPreparedThreadMessages.make(
            remoteMessages: remote,
            localMessages: local,
            preserveRemoteBeforeIndex: 40,
            isThreadBusy: false,
            activeAssistantMessageId: nil
        )

        XCTAssertEqual(
            prepared.messages.map(\.id),
            [
                "history:0",
                "history:40",
                "history:41",
                "origin:00000000-0000-0000-0000-000000000001",
            ]
        )
        XCTAssertNil(prepared.activeAssistantMessageId)
    }

    func testSelectedThreadUpdateUsesLocalRunTrackerBusyWithoutInventingRunState() {
        let transcript = GaryxThreadTranscript(
            ok: true,
            messages: [
                transcriptMessage(0, .user, "question"),
                transcriptMessage(1, .assistant, "answer"),
            ],
            pendingUserInputs: [],
            threadRuntime: nil,
            pageInfo: pageInfo(start: 0, end: 1)
        )

        let prepared = GaryxPreparedSelectedThreadTranscriptUpdate.make(
            from: transcript,
            localMessages: [],
            localRunTrackerBusy: true,
            activeAssistantMessageId: nil
        )

        XCTAssertFalse(prepared.runState.busy)
        XCTAssertTrue(prepared.threadRunActive)
        XCTAssertEqual(prepared.activitySignature, GaryxThreadActivitySignature.make(from: transcript))
        XCTAssertEqual(prepared.messages.messages.map(\.id), ["history:0", "history:1"])
    }

    func testSelectedThreadUpdateUsesReducedTranscriptRunState() {
        let transcript = GaryxThreadTranscript(
            ok: true,
            messages: [
                runStart(index: 0),
                transcriptMessage(1, .assistant, "working"),
            ],
            pendingUserInputs: [],
            threadRuntime: nil,
            pageInfo: pageInfo(start: 0, end: 1)
        )

        let prepared = GaryxPreparedSelectedThreadTranscriptUpdate.make(
            from: transcript,
            localMessages: [],
            localRunTrackerBusy: false,
            activeAssistantMessageId: "history:1"
        )

        XCTAssertTrue(prepared.runState.busy)
        XCTAssertTrue(prepared.threadRunActive)
        XCTAssertEqual(prepared.messages.activeAssistantMessageId, "history:1")
        XCTAssertEqual(prepared.messages.messages.map(\.id), ["history:1"])
        XCTAssertTrue(prepared.messages.messages[0].isStreaming)
    }

    func testSelectedThreadUpdateFromCachedWindowPreservesOlderLocalRows() {
        let window = GaryxCachedTranscript(
            threadId: "thread::test",
            savedAt: Date(timeIntervalSince1970: 0),
            messages: [
                transcriptMessage(40, .user, "new question"),
                transcriptMessage(41, .assistant, "new answer"),
            ],
            hasMoreBefore: true,
            nextBeforeIndex: 39
        )

        let prepared = GaryxPreparedSelectedThreadTranscriptUpdate.make(
            from: window,
            localMessages: [historyUser(0, text: "old question")],
            localRunTrackerBusy: false,
            activeAssistantMessageId: nil
        )

        XCTAssertFalse(prepared.threadRunActive)
        XCTAssertEqual(prepared.activitySignature, GaryxThreadActivitySignature.make(messages: window.messages, pendingUserInputs: []))
        XCTAssertEqual(prepared.messages.messages.map(\.id), ["history:0", "history:40", "history:41"])
    }

    func testMessageListSignatureTracksDisplayRelevantFieldsAndSampling() {
        let base = GaryxMessageListSignature.make(for: [mobileMessage("user-1", role: .user, text: "hello")])
        let changedText = GaryxMessageListSignature.make(for: [mobileMessage("user-1", role: .user, text: "goodbye")])
        XCTAssertNotEqual(base, changedText)

        var withStatus = mobileMessage("user-1", role: .user, text: "hello")
        withStatus.statusText = "Retry available"
        XCTAssertNotEqual(base, GaryxMessageListSignature.make(for: [withStatus]))

        var withAttachment = mobileMessage("user-1", role: .user, text: "hello")
        withAttachment.attachments = [
            GaryxMobileMessageAttachment(
                id: "attachment-1",
                kind: "image",
                name: "screenshot.png",
                mediaType: "image/png",
                remoteUrl: "https://example.test/image-a.png"
            ),
        ]
        var changedAttachment = withAttachment
        changedAttachment.attachments[0].remoteUrl = "https://example.test/image-b.png"
        XCTAssertNotEqual(
            GaryxMessageListSignature.make(for: [withAttachment]),
            GaryxMessageListSignature.make(for: [changedAttachment])
        )

        let longText = String(repeating: "x", count: 1_025)
        let sampled = GaryxMessageListSignature.make(for: [mobileMessage("user-1", role: .user, text: longText)])
        XCTAssertTrue(sampled.sampled)

        let runningTool = GaryxMessageListSignature.make(for: [toolMessage(status: .running)])
        let completedTool = GaryxMessageListSignature.make(for: [toolMessage(status: .completed)])
        XCTAssertNotEqual(runningTool, completedTool)
    }

    private func transcriptMessage(_ index: Int, _ role: GaryxTranscriptRole, _ text: String) -> GaryxTranscriptMessage {
        GaryxTranscriptMessage(index: index, role: role, text: text)
    }

    private func runStart(index: Int) -> GaryxTranscriptMessage {
        GaryxTranscriptMessage(
            index: index,
            role: .system,
            kind: "control",
            internalKind: "control",
            internalMessage: true,
            control: .object(["kind": .string("run_start"), "run_id": .string("run-1")]),
            likelyUserVisible: false
        )
    }

    private func pageInfo(start: Int, end: Int) -> GaryxThreadTranscriptPageInfo {
        GaryxThreadTranscriptPageInfo(
            returnedMessages: end - start + 1,
            returnedStartIndex: start,
            returnedEndIndex: end,
            hasMoreBefore: false,
            nextBeforeIndex: nil
        )
    }

    private func mobileMessage(
        _ id: String,
        role: GaryxMobileMessage.Role,
        text: String,
        historyIndex: Int? = nil
    ) -> GaryxMobileMessage {
        GaryxMobileMessage(
            id: id,
            role: role,
            text: text,
            timestamp: nil,
            isStreaming: false,
            localState: historyIndex == nil ? nil : .remoteFinal,
            historyIndex: historyIndex
        )
    }

    private func historyUser(_ index: Int, text: String) -> GaryxMobileMessage {
        mobileMessage("history:\(index)", role: .user, text: text, historyIndex: index)
    }

    private func historyAssistant(_ index: Int, text: String) -> GaryxMobileMessage {
        mobileMessage("history:\(index)", role: .assistant, text: text, historyIndex: index)
    }

    private func optimisticUser(_ id: String, text: String) -> GaryxMobileMessage {
        GaryxMobileMessage(
            id: id,
            role: .user,
            text: text,
            timestamp: nil,
            isStreaming: false,
            localState: .optimistic
        )
    }

    private func toolMessage(status: GaryxMobileToolTraceStatus) -> GaryxMobileMessage {
        let entry = GaryxMobileToolTraceEntry(
            id: "entry-1",
            toolUseId: "call-1",
            toolName: "exec_command",
            title: "Command",
            inputText: "swift test",
            inputLabel: "Call",
            resultLabel: "Result",
            status: status,
            isError: false,
            timestamp: nil,
            primaryPathBadge: nil
        )
        let group = GaryxMobileToolTraceGroup(entries: [entry], live: status == .running)
        return GaryxMobileMessage(
            id: "tool-group-1",
            role: .tool,
            text: group.summary,
            timestamp: nil,
            isStreaming: false,
            toolTraceGroup: group,
            localState: .remoteFinal,
            historyIndex: 2
        )
    }
}
