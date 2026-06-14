import XCTest
@testable import GaryxMobileCore

final class GaryxTaskNotificationPresentationTests: XCTestCase {
    func testParsesReadyForReviewNotificationAndOmitsReviewCommands() {
        let parsed = GaryxTaskNotificationPresentation.parse("""
        <garyx_task_notification event="ready_for_review" task_id="#TASK-42" status="in_review">
        Task #TASK-42 is ready for review: Ship task notifications

        Done.

        View details:
        garyx task get #TASK-42

        Review next:
        garyx task update #TASK-42 --status in_progress --note "needs changes: summary"
        garyx task update #TASK-42 --status done --note "approved by reviewer"
        </garyx_task_notification>
        """)

        XCTAssertEqual(parsed?.event, "ready_for_review")
        XCTAssertEqual(parsed?.status, "in_review")
        XCTAssertEqual(parsed?.taskId, "#TASK-42")
        XCTAssertEqual(parsed?.title, "Ship task notifications")
        XCTAssertEqual(parsed?.finalMessage, "Done.")
        XCTAssertFalse(parsed?.finalMessage.contains("garyx task update") ?? true)
    }

    func testKeepsMarkdownLikeFinalMessageWithChineseText() {
        let parsed = GaryxTaskNotificationPresentation.parse("""
        <garyx_task_notification event="ready_for_review" task_id="#TASK-528" status="in_review">
        Task #TASK-528 is ready for review: MCP tool review

        528(MCP) 已经跑完：

        - MCP manifest、tool discovery、enable/disable 都过了
        - 端到端验证覆盖了登录态 app 的真实调用路径
        - 和 527 的 sandboxAgentService/contracts 改动没有新冲突

        View details:
        garyx task get #TASK-528
        </garyx_task_notification>
        """)

        XCTAssertEqual(parsed?.taskId, "#TASK-528")
        XCTAssertEqual(parsed?.title, "MCP tool review")
        XCTAssertTrue(parsed?.finalMessage.contains("528(MCP) 已经跑完") ?? false)
        XCTAssertTrue(parsed?.finalMessage.contains("端到端验证覆盖了登录态 app") ?? false)
        XCTAssertFalse(parsed?.finalMessage.contains("garyx task get") ?? true)
    }

    func testIgnoresOrdinaryXMLSnippets() {
        XCTAssertNil(GaryxTaskNotificationPresentation.parse("<review>done</review>"))
    }

    func testStatusLabelFormatsKnownAndUnknownStates() {
        XCTAssertEqual(GaryxTaskNotificationPresentation.statusLabel(for: "in_review"), "In review")
        XCTAssertEqual(GaryxTaskNotificationPresentation.statusLabel(for: "needs-changes"), "Needs Changes")
    }
}
