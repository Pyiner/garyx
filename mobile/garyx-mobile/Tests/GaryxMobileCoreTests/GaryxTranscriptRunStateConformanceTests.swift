import XCTest
@testable import GaryxMobileCore

final class GaryxTranscriptRunStateConformanceTests: XCTestCase {
    private let decoder = JSONDecoder()

    private func fixtureURL(_ name: String) -> URL {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 {
            url.deleteLastPathComponent()
        }
        return url
            .appendingPathComponent("test-fixtures")
            .appendingPathComponent("stream-sync")
            .appendingPathComponent(name)
    }

    private func readJSONL(_ name: String) throws -> [[String: Any]] {
        let raw = try String(contentsOf: fixtureURL(name))
        return try raw
            .split(whereSeparator: \.isNewline)
            .map(String.init)
            .filter { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            .map { line in
                try XCTUnwrap(
                    JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any],
                    "\(name) line should decode as object"
                )
            }
    }

    private func decodeTranscriptMessage(fromRecord record: [String: Any]) throws -> GaryxTranscriptMessage {
        let rawMessage = try XCTUnwrap(record["message"] as? [String: Any], "record should carry message")
        let data = try JSONSerialization.data(withJSONObject: rawMessage)
        var message = try decoder.decode(GaryxTranscriptMessage.self, from: data)
        if let seq = record["seq"] as? Int {
            message.index = seq - 1
            message.id = "history:\(seq - 1)"
        }
        return message
    }

    private func controlMessage(seq: Int, event: [String: Any]) throws -> GaryxTranscriptMessage {
        let eventType = try XCTUnwrap(event["type"] as? String)
        let threadId = (event["thread_id"] as? String)
            ?? (event["threadId"] as? String)
            ?? "thread::fixture"
        let runId = (event["run_id"] as? String)
            ?? (event["runId"] as? String)
            ?? "run::fixture"
        var control: [String: Any] = [
            "kind": eventType,
            "thread_id": threadId,
            "run_id": runId,
            "at": "2026-06-18T12:00:00Z",
        ]
        if let pendingInputId = event["pending_input_id"] ?? event["pendingInputId"] {
            control["pending_input_id"] = pendingInputId
        }
        if let durationMs = event["duration_ms"] ?? event["durationMs"] {
            control["duration_ms"] = durationMs
        }
        if let title = event["title"] {
            control["title"] = title
        }
        return GaryxTranscriptMessage(
            index: seq - 1,
            role: .system,
            kind: "control",
            internalKind: "control",
            internalMessage: true,
            control: try jsonValue(control),
            likelyUserVisible: false
        )
    }

    private func jsonValue(_ object: [String: Any]) throws -> GaryxJSONValue {
        let data = try JSONSerialization.data(withJSONObject: object)
        return try decoder.decode(GaryxJSONValue.self, from: data)
    }

    func testKindResolverMatchesRustToolAndControlFixtureSemantics() throws {
        let messages = try readJSONL("transcript-with-tool.jsonl")
            .map(decodeTranscriptMessage(fromRecord:))
        XCTAssertEqual(messages.map { GaryxTranscriptKindResolver.kind(for: $0).rawValue }, [
            "user_input",
            "assistant_reply",
            "tool_trace",
            "tool_trace",
            "assistant_reply",
        ])

        let prose = GaryxTranscriptMessage(
            index: 0,
            role: .assistant,
            text: "this text mentions tool_use and mcp__ without a structured payload",
            content: .string("this text mentions tool_use and mcp__ without a structured payload")
        )
        XCTAssertEqual(GaryxTranscriptKindResolver.kind(for: prose), .assistantReply)

        let structuredTool = GaryxTranscriptMessage(
            index: 1,
            role: .assistant,
            content: .object(["tool_use_id": .string("call-1"), "input": .object([:])])
        )
        XCTAssertEqual(GaryxTranscriptKindResolver.kind(for: structuredTool), .toolTrace)

        let structuredToolInput = GaryxTranscriptMessage(
            index: 2,
            role: .assistant,
            input: .object(["tool_calls": .array([.object(["id": .string("call-2")])])])
        )
        XCTAssertEqual(GaryxTranscriptKindResolver.kind(for: structuredToolInput), .toolTrace)

        let structuredToolResult = GaryxTranscriptMessage(
            index: 3,
            role: .assistant,
            result: .object(["tool_use_id": .string("call-3")])
        )
        XCTAssertEqual(GaryxTranscriptKindResolver.kind(for: structuredToolResult), .toolTrace)

        let internalRole = try decoder.decode(GaryxTranscriptMessage.self, from: Data("""
        {"index":4,"role":"developer","text":"internal note"}
        """.utf8))
        XCTAssertEqual(GaryxTranscriptKindResolver.kind(for: internalRole), .internalMessage)

        let control = try controlMessage(seq: 1, event: [
            "type": "run_start",
            "threadId": "thread::kind",
        ])
        XCTAssertEqual(GaryxTranscriptKindResolver.kind(for: control), .control)
    }

    func testLifecycleFixtureReplaysToRustGoldenIdleTerminalState() throws {
        let events = try readJSONL("stream-lifecycle.jsonl")
        var records: [GaryxTranscriptMessage] = []
        for event in events {
            if event["type"] as? String == "committed_message" {
                records.append(try decodeTranscriptMessage(fromRecord: event))
            }
        }

        let state = GaryxTranscriptRunStateReducer.reduce(records)
        XCTAssertFalse(state.busy)
        XCTAssertNil(state.activeRunId)
        XCTAssertEqual(state.activity, .idle)
        XCTAssertEqual(state.terminalStatus, "completed")
    }

    func testUserAckFixtureReplaysAckPositionAndToolActivity() throws {
        let events = try readJSONL("stream-events-with-user-ack.jsonl")
        var records: [GaryxTranscriptMessage] = []
        for event in events {
            if event["type"] as? String == "committed_message" {
                records.append(try decodeTranscriptMessage(fromRecord: event))
            }
        }

        let state = GaryxTranscriptRunStateReducer.reduce(records)
        XCTAssertFalse(state.busy)
        XCTAssertEqual(state.activity, .idle)
        XCTAssertEqual(state.terminalStatus, "completed")
        XCTAssertEqual(state.lastUserAckPendingInputId, "pending-fixture-followup")
        XCTAssertEqual(state.lastUserAckSeq, 4)
    }

    func testMultiToolLullFixtureReplaysFinishedToolGapAsThinking() throws {
        let records = try readJSONL("multi-tool-lull.jsonl")
            .filter { $0["type"] as? String == "committed_message" }
            .map(decodeTranscriptMessage(fromRecord:))

        let firstToolLull = GaryxTranscriptRunStateReducer.reduce(records.filter { ($0.index ?? -1) <= 3 })
        XCTAssertTrue(firstToolLull.busy)
        XCTAssertEqual(firstToolLull.activity, .thinking)

        let secondToolRunning = GaryxTranscriptRunStateReducer.reduce(records.filter { ($0.index ?? -1) <= 4 })
        XCTAssertTrue(secondToolRunning.busy)
        XCTAssertEqual(secondToolRunning.activity, .usingTool)

        let finalToolLull = GaryxTranscriptRunStateReducer.reduce(records.filter { ($0.index ?? -1) <= 5 })
        XCTAssertTrue(finalToolLull.busy)
        XCTAssertEqual(finalToolLull.activity, .thinking)
    }

    func testParallelToolLullFixtureWaitsForAllResultsBeforeThinking() throws {
        let records = try readJSONL("parallel-tool-lull.jsonl")
            .filter { $0["type"] as? String == "committed_message" }
            .map(decodeTranscriptMessage(fromRecord:))

        let bothToolsRunning = GaryxTranscriptRunStateReducer.reduce(records.filter { ($0.index ?? -1) <= 3 })
        XCTAssertTrue(bothToolsRunning.busy)
        XCTAssertEqual(bothToolsRunning.activity, .usingTool)

        let oneToolStillRunning = GaryxTranscriptRunStateReducer.reduce(records.filter { ($0.index ?? -1) <= 4 })
        XCTAssertTrue(oneToolStillRunning.busy)
        XCTAssertEqual(oneToolStillRunning.activity, .usingTool)

        let allToolsFinished = GaryxTranscriptRunStateReducer.reduce(records.filter { ($0.index ?? -1) <= 5 })
        XCTAssertTrue(allToolsFinished.busy)
        XCTAssertEqual(allToolsFinished.activity, .thinking)
    }

    func testToolResultDetectionMatchesClientEdgeCases() throws {
        let runStart = try controlMessage(seq: 1, event: [
            "type": "run_start",
            "threadId": "thread::fixture-tool-result-edges",
            "runId": "run::fixture-tool-result-edges",
        ])
        let nullResultToolUse = GaryxTranscriptMessage(
            index: 1,
            role: .toolUse,
            kind: "tool_trace",
            content: .object([
                "type": .string("commandExecution"),
                "id": .string("call_fixture_null_result"),
            ]),
            result: .null,
            toolUseId: "call_fixture_null_result"
        )
        let nullResultState = GaryxTranscriptRunStateReducer.reduce([runStart, nullResultToolUse])
        XCTAssertTrue(nullResultState.busy)
        XCTAssertEqual(nullResultState.activity, .usingTool)

        let kindResultTrace = GaryxTranscriptMessage(
            index: 1,
            role: .toolUse,
            kind: "tool_trace",
            content: .object([
                "type": .string("commandExecution"),
                "kind": .string("tool_result"),
                "id": .string("call_fixture_kind_result"),
            ]),
            toolUseId: "call_fixture_kind_result"
        )
        let kindResultState = GaryxTranscriptRunStateReducer.reduce([runStart, kindResultTrace])
        XCTAssertTrue(kindResultState.busy)
        XCTAssertEqual(kindResultState.activity, .thinking)
    }

    func testTranscriptWithControlFixtureReplaysCommittedControlState() throws {
        let records = try readJSONL("transcript-with-control.jsonl")
            .map(decodeTranscriptMessage(fromRecord:))
        let state = GaryxTranscriptRunStateReducer.reduce(records)
        XCTAssertTrue(state.busy)
        XCTAssertEqual(state.activeRunId, "run::fixture-control")
        XCTAssertEqual(state.activity, .reconciling)
        XCTAssertNil(state.terminalStatus)
    }

    func testRewriteControlsSurfaceReplayInvalidationWindows() throws {
        let records = [
            try controlMessage(seq: 1, event: [
                "type": "run_start",
                "threadId": "thread::fixture-rewrite",
                "runId": "run::fixture-rewrite",
            ]),
            GaryxTranscriptMessage(
                index: 1,
                role: .system,
                kind: "control",
                internalKind: "control",
                internalMessage: true,
                control: .object([
                    "kind": .string("range_rewrite"),
                    "start_seq": .number(1),
                    "end_seq": .number(1),
                ]),
                likelyUserVisible: false
            ),
            GaryxTranscriptMessage(
                index: 2,
                role: .system,
                kind: "control",
                internalKind: "control",
                internalMessage: true,
                control: .object(["kind": .string("transcript_reset")]),
                likelyUserVisible: false
            ),
        ]
        let state = GaryxTranscriptRunStateReducer.reduce(records)
        XCTAssertEqual(state.rewriteRanges, [
            GaryxTranscriptRewriteRange(noticeSeq: 2, startSeq: 1, endSeq: 1),
        ])
        XCTAssertEqual(state.lastTranscriptResetSeq, 3)
    }
}
