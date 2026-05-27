import XCTest
@testable import GaryxMobileCore

final class GaryxMobileThreadOpenStateTests: XCTestCase {
    func testQueueTrimsThreadIdAndOwnsRequest() {
        var state = GaryxMobileThreadOpenState(requestId: UUID(uuidString: "00000000-0000-0000-0000-000000000001")!)
        let requestId = UUID(uuidString: "00000000-0000-0000-0000-000000000002")!

        XCTAssertEqual(state.queue(threadId: " thread::abc ", source: .url, requestId: requestId), requestId)

        XCTAssertEqual(state.pendingThreadId, "thread::abc")
        XCTAssertEqual(state.pendingSource, .url)
        XCTAssertEqual(state.requestId, requestId)
        XCTAssertTrue(state.hasPendingIntent)
    }

    func testQueueRejectsEmptyThreadIdWithoutChangingExistingIntent() {
        var state = GaryxMobileThreadOpenState()
        let requestId = UUID(uuidString: "00000000-0000-0000-0000-000000000003")!
        _ = state.queue(threadId: "thread::abc", source: .url, requestId: requestId)

        XCTAssertNil(state.queue(threadId: "   ", source: .url, requestId: UUID()))

        XCTAssertEqual(state.pendingThreadId, "thread::abc")
        XCTAssertEqual(state.requestId, requestId)
    }

    func testNewIntentSupersedesOldRequest() {
        var state = GaryxMobileThreadOpenState()
        let oldRequestId = UUID(uuidString: "00000000-0000-0000-0000-000000000004")!
        let newRequestId = UUID(uuidString: "00000000-0000-0000-0000-000000000005")!
        _ = state.queue(threadId: "thread::old", source: .url, requestId: oldRequestId)

        _ = state.queue(threadId: "thread::new", source: .url, requestId: newRequestId)

        XCTAssertFalse(state.isCurrent(oldRequestId))
        XCTAssertTrue(state.isCurrent(newRequestId))
        XCTAssertEqual(state.pendingThreadId, "thread::new")
    }

    func testMarkShownRequiresCurrentRequestAndPendingThread() {
        var state = GaryxMobileThreadOpenState()
        let requestId = UUID(uuidString: "00000000-0000-0000-0000-000000000006")!
        _ = state.queue(threadId: "thread::abc", source: .url, requestId: requestId)

        XCTAssertFalse(state.markShown(threadId: "thread::abc", requestId: UUID()))
        XCTAssertNil(state.shownThreadId)
        XCTAssertFalse(state.markShown(threadId: "thread::other", requestId: requestId))
        XCTAssertNil(state.shownThreadId)
        XCTAssertTrue(state.markShown(threadId: "thread::abc", requestId: requestId))
        XCTAssertEqual(state.shownThreadId, "thread::abc")
    }

    func testCompleteClearsOnlyCurrentPendingIntent() {
        var state = GaryxMobileThreadOpenState()
        let requestId = UUID(uuidString: "00000000-0000-0000-0000-000000000007")!
        _ = state.queue(threadId: "thread::abc", source: .url, requestId: requestId)
        _ = state.markShown(threadId: "thread::abc", requestId: requestId)

        XCTAssertFalse(state.complete(threadId: "thread::abc", requestId: UUID()))
        XCTAssertEqual(state.pendingThreadId, "thread::abc")
        XCTAssertTrue(state.complete(threadId: "thread::abc", requestId: requestId))
        XCTAssertNil(state.pendingThreadId)
        XCTAssertNil(state.pendingSource)
        XCTAssertNil(state.shownThreadId)
    }

    func testDirectOpenInvalidatesPendingIntentButKeepsOwnershipToken() {
        var state = GaryxMobileThreadOpenState()
        _ = state.queue(threadId: "thread::abc", source: .url, requestId: UUID())
        let directRequestId = UUID(uuidString: "00000000-0000-0000-0000-000000000008")!

        XCTAssertEqual(state.beginDirectOpen(requestId: directRequestId), directRequestId)

        XCTAssertFalse(state.hasPendingIntent)
        XCTAssertNil(state.pendingThreadId)
        XCTAssertEqual(state.pendingSource, .direct)
        XCTAssertTrue(state.isCurrent(directRequestId))
    }
}
