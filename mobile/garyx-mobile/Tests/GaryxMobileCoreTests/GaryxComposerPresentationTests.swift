import XCTest
@testable import GaryxMobileCore

/// #TASK-1453 problem A — the chat composer's placeholder + action buttons must
/// track the thread's real run state, not whether a thread is open and not the
/// tail transcript row. An idle thread (including one whose tail row is a
/// capsule card) shows the prompt placeholder + send button; only an active run
/// shows the follow-up placeholder + stop button.
final class GaryxComposerPresentationTests: XCTestCase {
    func testIdleThreadShowsPromptAndSendNotStop() {
        // The screenshot scenario: idle chat thread, tail row is a capsule card.
        // The tail row is deliberately not an input — run state is idle.
        let p = GaryxComposerPresentationResolver.resolve(isThreadBusy: false, hasLocalPayload: false)
        XCTAssertEqual(p.placeholder, .prompt, "idle composer must not show the follow-up placeholder")
        XCTAssertFalse(p.showsStopButton, "idle composer must not show the stop button")
        XCTAssertTrue(p.showsSendButton)
    }

    func testIdleThreadWithDraftShowsSend() {
        let p = GaryxComposerPresentationResolver.resolve(isThreadBusy: false, hasLocalPayload: true)
        XCTAssertEqual(p.placeholder, .prompt)
        XCTAssertFalse(p.showsStopButton)
        XCTAssertTrue(p.showsSendButton)
    }

    func testBusyThreadShowsFollowUpAndStop() {
        let p = GaryxComposerPresentationResolver.resolve(isThreadBusy: true, hasLocalPayload: false)
        XCTAssertEqual(p.placeholder, .followUp, "an active run shows the follow-up placeholder")
        XCTAssertTrue(p.showsStopButton, "an active run shows the stop button")
        XCTAssertFalse(p.showsSendButton, "with no draft and a run in flight, only stop is offered")
    }

    func testBusyThreadWithDraftKeepsSendForQueuedFollowUp() {
        let p = GaryxComposerPresentationResolver.resolve(isThreadBusy: true, hasLocalPayload: true)
        XCTAssertEqual(p.placeholder, .followUp)
        XCTAssertTrue(p.showsStopButton)
        XCTAssertTrue(p.showsSendButton, "a drafted follow-up can be queued while busy")
    }
}
