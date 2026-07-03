import XCTest
@testable import GaryxMobileCore

final class GaryxThreadTranscriptSnapshotPayloadTests: XCTestCase {
    func testDecodesTranscriptFromObjectPayload() throws {
        let payload: [String: GaryxJSONValue] = [
            "payload": .object([
                "ok": .bool(true),
                "messages": .array([
                    .object([
                        "role": .string("assistant"),
                        "text": .string("Hello"),
                        "index": .number(4),
                    ]),
                ]),
                "pending_user_inputs": .array([]),
            ]),
        ]
        let transcript = try XCTUnwrap(GaryxThreadTranscript.fromSnapshotPayload(payload))
        XCTAssertTrue(transcript.ok)
        XCTAssertEqual(transcript.messages.count, 1)
        XCTAssertEqual(transcript.messages.first?.text, "Hello")
        XCTAssertEqual(transcript.messages.first?.index, 4)
        XCTAssertEqual(transcript.messages.first?.role, .assistant)
    }

    func testMissingPayloadKeyReturnsNil() throws {
        XCTAssertNil(try GaryxThreadTranscript.fromSnapshotPayload([:]))
        XCTAssertNil(try GaryxThreadTranscript.fromSnapshotPayload(["other": .object([:])]))
    }

    func testNonObjectPayloadReturnsNil() throws {
        XCTAssertNil(try GaryxThreadTranscript.fromSnapshotPayload(["payload": .string("nope")]))
        XCTAssertNil(try GaryxThreadTranscript.fromSnapshotPayload(["payload": .null]))
    }
}
