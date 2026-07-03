import XCTest
@testable import GaryxMobileCore

final class GaryxSelectedThreadRecoveryPolicyTests: XCTestCase {
    func testContinuesWhileSelectedAndRemoteBusy() {
        XCTAssertTrue(
            GaryxSelectedThreadRecoveryPolicy.shouldContinueRecovering(
                threadId: "t-1",
                selectedThreadId: "t-1",
                remoteBusyThreadIds: ["t-1", "t-2"]
            )
        )
    }

    func testStopsWhenSelectionChanged() {
        XCTAssertFalse(
            GaryxSelectedThreadRecoveryPolicy.shouldContinueRecovering(
                threadId: "t-1",
                selectedThreadId: "t-2",
                remoteBusyThreadIds: ["t-1"]
            )
        )
    }

    func testStopsWhenNothingSelected() {
        XCTAssertFalse(
            GaryxSelectedThreadRecoveryPolicy.shouldContinueRecovering(
                threadId: "t-1",
                selectedThreadId: nil,
                remoteBusyThreadIds: ["t-1"]
            )
        )
    }

    func testStopsWhenThreadNoLongerRemoteBusy() {
        XCTAssertFalse(
            GaryxSelectedThreadRecoveryPolicy.shouldContinueRecovering(
                threadId: "t-1",
                selectedThreadId: "t-1",
                remoteBusyThreadIds: []
            )
        )
    }
}
