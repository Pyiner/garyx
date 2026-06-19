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

    private func contentRecord(from event: [String: Any], seq: Int) throws -> GaryxTranscriptMessage {
        let threadId = (event["thread_id"] as? String)
            ?? (event["threadId"] as? String)
            ?? "thread::fixture"
        var record = event
        if record["message"] == nil {
            record["message"] = event
        }
        record["seq"] = seq
        record["thread_id"] = threadId
        return try decodeTranscriptMessage(fromRecord: record)
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

        let control = try controlMessage(seq: 1, event: [
            "type": "run_start",
            "threadId": "thread::kind",
        ])
        XCTAssertEqual(GaryxTranscriptKindResolver.kind(for: control), .control)
    }

    func testLifecycleFixtureReplaysToRustGoldenIdleTerminalState() throws {
        let events = try readJSONL("stream-lifecycle.jsonl")
        var records: [GaryxTranscriptMessage] = []
        var nextSeq = 1
        for event in events {
            switch event["type"] as? String {
            case "committed_message":
                records.append(try contentRecord(from: event, seq: nextSeq))
                nextSeq += 1
            case "run_start", "done", "run_complete":
                records.append(try controlMessage(seq: nextSeq, event: event))
                nextSeq += 1
            default:
                break
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
        var records = [
            try controlMessage(seq: 1, event: [
                "type": "run_start",
                "threadId": "thread::fixture-stream-sync-ack",
                "runId": "run::fixture-ack",
            ]),
        ]
        var nextSeq = 2
        for event in events {
            switch event["type"] as? String {
            case "tool_use", "tool_result":
                records.append(try contentRecord(from: event, seq: nextSeq))
                nextSeq += 1
            case "user_ack", "assistant_boundary", "done":
                records.append(try controlMessage(seq: nextSeq, event: event))
                nextSeq += 1
            default:
                break
            }
        }

        let state = GaryxTranscriptRunStateReducer.reduce(records)
        XCTAssertTrue(state.busy, "fixture has done but no run_complete terminal")
        XCTAssertEqual(state.activity, .reconciling)
        XCTAssertEqual(state.lastUserAckPendingInputId, "pending-fixture-followup")
        XCTAssertEqual(state.lastUserAckSeq, 3)
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
