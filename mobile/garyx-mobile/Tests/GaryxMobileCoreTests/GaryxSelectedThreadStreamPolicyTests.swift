import XCTest
@testable import GaryxMobileCore

final class GaryxSelectedThreadStreamPolicyTests: XCTestCase {
    func testNewSelectedThreadStartsPerThreadStream() {
        XCTAssertEqual(
            GaryxSelectedThreadStreamPolicy.action(previousThreadId: nil, selectedThreadId: "thread-new"),
            .start("thread-new")
        )
    }

    func testVisibleConversationRestartsStreamAfterHomeStop() {
        XCTAssertTrue(GaryxVisibleConversationStreamPolicy.shouldStart(
            isConversationVisible: true,
            selectedThreadId: "thread-current",
            streamOwnedThreadId: nil,
            hasStreamTask: false
        ))
    }

    func testVisibleConversationDoesNotRestartAlreadyOwnedStream() {
        XCTAssertFalse(GaryxVisibleConversationStreamPolicy.shouldStart(
            isConversationVisible: true,
            selectedThreadId: "thread-current",
            streamOwnedThreadId: "thread-current",
            hasStreamTask: true
        ))
    }

    func testHiddenHomeSurfaceDoesNotStartSelectedThreadStream() {
        XCTAssertFalse(GaryxVisibleConversationStreamPolicy.shouldStart(
            isConversationVisible: false,
            selectedThreadId: "thread-current",
            streamOwnedThreadId: nil,
            hasStreamTask: false
        ))
    }
}
