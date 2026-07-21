import XCTest
@testable import GaryxMobileCore

final class GaryxTaskNotificationPresentationTests: XCTestCase {
    private let presentation = GaryxRenderMessagePresentation.taskNotification(
        event: "ready_for_review",
        status: "in_review",
        taskId: "#TASK-42",
        title: "Ship task notifications"
    )

    func testStructurallyStripsOnlyEnvelopeBody() {
        let text = """
        <garyx_task_notification event="ready_for_review" task_id="#TASK-42" status="in_review" title="Changed wording">
        Done with wording that has no prose-parser sentinel.

        - Markdown stays intact.
        </garyx_task_notification>

        View details: garyx task get #TASK-42
        Review next: tutorial stays outside.
        """

        XCTAssertEqual(
            GaryxTaskNotificationPresentation.stripEnvelope(from: text),
            "Done with wording that has no prose-parser sentinel.\n\n- Markdown stays intact."
        )
    }

    func testUsesLastCloseTagForLegacyBodies() {
        let text = """
        <garyx_task_notification event="ready_for_review">
        Body with neutralized </garyx_task_notification > text.
        Legacy tutorial wording remains readable.
        </garyx_task_notification>
        """

        XCTAssertEqual(
            GaryxTaskNotificationPresentation.stripEnvelope(from: text),
            "Body with neutralized </garyx_task_notification > text.\nLegacy tutorial wording remains readable."
        )
        XCTAssertNil(GaryxTaskNotificationPresentation.stripEnvelope(from: "<review>done</review>"))
    }

    func testMessagePresentationUsesServerPayloadInsteadOfInferringFromText() {
        let text = """
        <garyx_task_notification task_id="#TASK-42">
        Ready without the old English sentence.
        </garyx_task_notification>
        """
        let ordinary = GaryxMobileMessage(
            id: "ordinary-user",
            role: .user,
            text: text,
            timestamp: nil,
            isStreaming: false
        )
        XCTAssertEqual(GaryxMobileMessagePresentation.make(for: ordinary), .text(text))

        var classified = ordinary
        classified.renderPresentation = presentation
        guard case let .taskNotification(_, notification) = GaryxMobileMessagePresentation.make(for: classified) else {
            return XCTFail("server presentation must select the card before envelope decoding")
        }
        XCTAssertEqual(
            notification,
            GaryxTaskNotification(
                event: "ready_for_review",
                status: "in_review",
                taskId: "#TASK-42",
                title: "Ship task notifications",
                finalMessage: "Ready without the old English sentence."
            )
        )
    }

    func testMalformedEnvelopeFallsBackToOrdinaryText() {
        var message = GaryxMobileMessage(
            id: "malformed-user",
            role: .user,
            text: "<garyx_task_notification>missing close",
            timestamp: nil,
            isStreaming: false
        )
        message.renderPresentation = presentation
        XCTAssertEqual(GaryxMobileMessagePresentation.make(for: message), .text(message.text))
    }

    func testStatusLabelFormatsKnownAndUnknownStates() {
        XCTAssertEqual(GaryxTaskNotificationPresentation.statusLabel(for: "in_review"), "In review")
        XCTAssertEqual(GaryxTaskNotificationPresentation.statusLabel(for: "needs-changes"), "Needs Changes")
    }

    func testOverflowDecisionHonorsInjectedEpsilon() {
        XCTAssertFalse(GaryxTaskNotificationOverflow.overflows(
            naturalHeight: 200,
            clampHeight: 200,
            epsilon: 0.5
        ))
        XCTAssertFalse(GaryxTaskNotificationOverflow.overflows(
            naturalHeight: 200.5,
            clampHeight: 200,
            epsilon: 0.5
        ))
        XCTAssertTrue(GaryxTaskNotificationOverflow.overflows(
            naturalHeight: 200.5001,
            clampHeight: 200,
            epsilon: 0.5
        ))
    }

    func testSelectionSnapshotSurvivesRowEvictionAndClearsOnEveryScopeChange() {
        let notification = GaryxTaskNotification(
            event: "ready_for_review",
            status: "in_review",
            taskId: "#TASK-42",
            title: "Immutable title",
            finalMessage: "Complete immutable body"
        )
        let selection = GaryxTaskNotificationSelection(
            messageId: "seq:42",
            messageSeq: 42,
            notification: notification
        )
        let initial = GaryxTaskNotificationPresentationScope(
            threadIdentity: "thread-a",
            gatewayIdentity: "gateway-a",
            occurrenceIdentity: "occurrence-a"
        )
        var state = GaryxTaskNotificationSelectionState()
        state.present(selection, scope: initial)

        // Selection owns its complete value snapshot; there is deliberately no
        // row lookup to invalidate when the server render window evicts it.
        XCTAssertEqual(state.selection?.notification, notification)

        for changed in [
            GaryxTaskNotificationPresentationScope(
                threadIdentity: "thread-b",
                gatewayIdentity: "gateway-a",
                occurrenceIdentity: "occurrence-a"
            ),
            GaryxTaskNotificationPresentationScope(
                threadIdentity: "thread-a",
                gatewayIdentity: "gateway-b",
                occurrenceIdentity: "occurrence-a"
            ),
            GaryxTaskNotificationPresentationScope(
                threadIdentity: "thread-a",
                gatewayIdentity: "gateway-a",
                occurrenceIdentity: "occurrence-b"
            ),
        ] {
            state.present(selection, scope: initial)
            XCTAssertTrue(state.synchronize(scope: changed))
            XCTAssertNil(state.selection)
        }
    }
}
