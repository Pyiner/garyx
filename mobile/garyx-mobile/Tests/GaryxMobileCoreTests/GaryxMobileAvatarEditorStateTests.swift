import XCTest
@testable import GaryxMobileCore

final class GaryxMobileAvatarEditorStateTests: XCTestCase {
    func testBeginTracksActivityFingerprintAndRequest() {
        var state = GaryxMobileAvatarEditorState()
        let requestId = UUID(uuidString: "00000000-0000-0000-0000-000000000101")!

        XCTAssertEqual(state.begin(.generate, fingerprint: "agent-a", requestId: requestId), requestId)

        XCTAssertTrue(state.isBusy)
        XCTAssertTrue(state.isGenerating)
        XCTAssertFalse(state.isUploading)
        XCTAssertTrue(state.canApply(requestId: requestId, fingerprint: "agent-a"))
        XCTAssertFalse(state.canApply(requestId: requestId, fingerprint: "agent-b"))
    }

    func testNewRequestSupersedesOldRequest() {
        var state = GaryxMobileAvatarEditorState()
        let oldRequestId = UUID(uuidString: "00000000-0000-0000-0000-000000000102")!
        let newRequestId = UUID(uuidString: "00000000-0000-0000-0000-000000000103")!
        _ = state.begin(.generate, fingerprint: "first", requestId: oldRequestId)

        _ = state.begin(.upload, fingerprint: "second", requestId: newRequestId)

        XCTAssertFalse(state.canApply(requestId: oldRequestId, fingerprint: "first"))
        XCTAssertTrue(state.canApply(requestId: newRequestId, fingerprint: "second"))
        XCTAssertFalse(state.isGenerating)
        XCTAssertTrue(state.isUploading)
    }

    func testFinishOnlyClearsCurrentRequest() {
        var state = GaryxMobileAvatarEditorState()
        let requestId = UUID(uuidString: "00000000-0000-0000-0000-000000000104")!
        _ = state.begin(.generate, fingerprint: "agent-a", requestId: requestId)

        state.finish(requestId: UUID())
        XCTAssertTrue(state.isBusy)

        state.finish(requestId: requestId)
        XCTAssertFalse(state.isBusy)
        XCTAssertNil(state.requestId)
        XCTAssertEqual(state.fingerprint, "")
    }

    func testResetClearsRequestAndRejectsPriorApply() {
        var state = GaryxMobileAvatarEditorState()
        let requestId = UUID(uuidString: "00000000-0000-0000-0000-000000000105")!
        _ = state.begin(.upload, fingerprint: "avatar-a", requestId: requestId)

        state.reset()

        XCTAssertFalse(state.isBusy)
        XCTAssertNil(state.requestId)
        XCTAssertFalse(state.canApply(requestId: requestId, fingerprint: "avatar-a"))
    }
}
