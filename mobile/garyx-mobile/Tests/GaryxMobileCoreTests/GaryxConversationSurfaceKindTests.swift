import XCTest
@testable import GaryxMobileCore

/// Acceptance tests for #TASK-1449 symptom 1: the conversation surface kind is a
/// pure function of the thread's objective `thread_type`, independent of entry
/// path; an unclassified by-id open is chat-loading, never the workflow surface.
final class GaryxConversationSurfaceKindTests: XCTestCase {
    func testChatThreadResolvesToChatRegardlessOfEntryPath() {
        XCTAssertEqual(
            GaryxConversationSurfaceKind.resolve(summary: chatSummary("thread::T"), isResolvingById: false),
            .chat
        )
        XCTAssertEqual(
            GaryxConversationSurfaceKind.resolve(summary: chatSummary("thread::T"), isResolvingById: true),
            .chat
        )
    }

    func testWorkflowRunThreadIsTheOnlyWorkflowSurface() {
        XCTAssertEqual(
            GaryxConversationSurfaceKind.resolve(
                summary: workflowSummary("thread::W", runId: "wfr::1"),
                isResolvingById: false
            ),
            .workflowRun(runId: "wfr::1")
        )
    }

    func testByIdOpenWithoutSummaryIsLoadingUnknownNotWorkflow() {
        let kind = GaryxConversationSurfaceKind.resolve(summary: nil, isResolvingById: true)
        XCTAssertEqual(kind, .loadingUnknown)
        XCTAssertFalse(kind.presentsWorkflowRun, "an unclassified by-id open must not present the workflow surface")
    }

    func testPlainDraftWithoutSummaryIsChatComposer() {
        XCTAssertEqual(GaryxConversationSurfaceKind.resolve(summary: nil, isResolvingById: false), .chat)
    }

    func testUnstampedThreadTypeNeverPresentsWorkflow() {
        // A summary whose thread_type the gateway has not stamped (empty) is
        // `.unresolved`; while resolving by id it is chat-loading, never workflow.
        let summary = summary(id: "thread::U", threadType: "", workflowRunId: nil)
        XCTAssertEqual(
            GaryxConversationSurfaceKind.resolve(summary: summary, isResolvingById: true),
            .loadingUnknown
        )
        XCTAssertFalse(
            GaryxConversationSurfaceKind.resolve(summary: summary, isResolvingById: true).presentsWorkflowRun
        )
    }

    // MARK: Fixtures

    private func chatSummary(_ id: String) -> GaryxThreadSummary {
        summary(id: id, threadType: "chat", workflowRunId: nil)
    }

    private func workflowSummary(_ id: String, runId: String) -> GaryxThreadSummary {
        summary(id: id, threadType: "workflow_run", workflowRunId: runId)
    }

    private func summary(id: String, threadType: String, workflowRunId: String?) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: id,
            title: "Test Thread",
            createdAt: nil,
            updatedAt: nil,
            lastMessagePreview: "",
            workspacePath: nil,
            messageCount: nil,
            agentId: nil,
            providerType: nil,
            recentRunId: nil,
            activeRunId: nil,
            runState: nil,
            worktreePath: nil,
            threadType: threadType,
            workflowRunId: workflowRunId
        )
    }
}
