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
