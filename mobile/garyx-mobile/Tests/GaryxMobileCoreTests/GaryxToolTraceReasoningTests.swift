import XCTest
@testable import GaryxMobileCore

final class GaryxToolTraceReasoningTests: XCTestCase {
    private func json(_ raw: String) -> GaryxJSONValue {
        try! JSONDecoder().decode(GaryxJSONValue.self, from: Data(raw.utf8))
    }

    func testReasoningRecordsAreDetectedAcrossShapes() {
        // Codex stores chain-of-thought as a reasoning tool record with empty
        // content; the client skips it so it is not rendered as a "Used 1 tool".
        let byContentType = GaryxTranscriptMessage(
            index: 1,
            role: .toolUse,
            content: json(#"{"type":"reasoning","id":"rs_1","content":[],"summary":[]}"#)
        )
        XCTAssertTrue(GaryxMobileTranscriptToolTraceClassifier.isReasoningTrace(byContentType))

        let byKind = GaryxTranscriptMessage(index: 2, role: .toolUse, kind: "reasoning")
        XCTAssertTrue(GaryxMobileTranscriptToolTraceClassifier.isReasoningTrace(byKind))

        let byToolName = GaryxTranscriptMessage(
            index: 3,
            role: .toolResult,
            content: json(#"{"tool_name":"reasoning"}"#)
        )
        XCTAssertTrue(GaryxMobileTranscriptToolTraceClassifier.isReasoningTrace(byToolName))
    }

    func testGaryxParentToolUseIdExtractedFromEnvelopeMetadata() {
        // A sub-agent's nested tool call carries parent_tool_use_id in the
        // envelope metadata (the nested content omits it). The mobile client uses
        // this to filter sub-agent tool calls, keeping only the Agent spawn.
        let child = GaryxTranscriptMessage(
            index: 1,
            role: .toolUse,
            content: json(#"{"input":{"command":"ls"},"tool":"Bash"}"#),
            metadata: json(#"{"parent_tool_use_id":"toolu_PARENT","source":"claude_sdk"}"#)
        )
        XCTAssertEqual(child.garyxParentToolUseId, "toolu_PARENT")

        let agentSpawn = GaryxTranscriptMessage(
            index: 2,
            role: .toolUse,
            content: json(#"{"input":{},"tool":"Agent"}"#),
            metadata: json(#"{"source":"claude_sdk"}"#)
        )
        XCTAssertNil(agentSpawn.garyxParentToolUseId, "the Agent spawn is top-level and must be kept")

        XCTAssertNil(GaryxTranscriptMessage(index: 3, role: .toolUse).garyxParentToolUseId)
    }

    func testEnvelopeIdentityRoundTripsThroughCacheCodec() throws {
        // The on-device transcript cache re-encodes/decodes committed rows; the
        // newly-decoded envelope identity must survive so cached rows still
        // filter sub-agent children and dedup correctly.
        let original = GaryxTranscriptMessage(
            index: 7,
            role: .toolUse,
            content: json(#"{"input":{"command":"ls"},"tool":"Bash"}"#),
            toolUseId: "toolu_X",
            metadata: json(#"{"parent_tool_use_id":"toolu_PARENT","source":"claude_sdk"}"#)
        )
        let data = try JSONEncoder().encode(original)
        let decoded = try JSONDecoder().decode(GaryxTranscriptMessage.self, from: data)
        XCTAssertEqual(decoded.toolUseId, "toolu_X")
        XCTAssertEqual(decoded.garyxParentToolUseId, "toolu_PARENT")
    }

    func testRealToolCallsAreNotTreatedAsReasoning() {
        let command = GaryxTranscriptMessage(
            index: 1,
            role: .toolUse,
            content: json(#"{"type":"local_shell_call","command":"ls","tool_name":"commandExecution"}"#)
        )
        XCTAssertFalse(GaryxMobileTranscriptToolTraceClassifier.isReasoningTrace(command))

        let output = GaryxTranscriptMessage(
            index: 2,
            role: .toolResult,
            content: json(#"{"type":"local_shell_call_output","output":"done"}"#)
        )
        XCTAssertFalse(GaryxMobileTranscriptToolTraceClassifier.isReasoningTrace(output))
    }
}
