import XCTest
@testable import GaryxMobileCore

final class GaryxMobilePresentationModelsTests: XCTestCase {
    func testRunStateResolverPreservesAPIRunningWhenCommittedStateIsMissing() {
        XCTAssertEqual(
            GaryxThreadSummaryRunStateResolver.resolvedRunState(
                apiRunState: "running",
                recentRunId: nil,
                committedState: nil
            ),
            "running"
        )
        XCTAssertEqual(
            GaryxThreadSummaryRunStateResolver.resolvedRunState(
                apiRunState: " running ",
                recentRunId: "run-1",
                committedState: nil
            ),
            " running "
        )
    }

    func testRunStateResolverUsesCommittedTerminalStateWhenPresent() {
        let state = GaryxTranscriptRunState(
            busy: false,
            activeRunId: nil,
            terminalStatus: "completed"
        )

        XCTAssertEqual(
            GaryxThreadSummaryRunStateResolver.resolvedRunState(
                apiRunState: "running",
                recentRunId: "run-1",
                committedState: state
            ),
            "completed"
        )
    }

    func testRunStateResolverFallsBackFromCommittedIdleByRecentRun() {
        XCTAssertEqual(
            GaryxThreadSummaryRunStateResolver.resolvedRunState(
                apiRunState: "running",
                recentRunId: "run-1",
                committedState: GaryxTranscriptRunState()
            ),
            "completed"
        )
        XCTAssertEqual(
            GaryxThreadSummaryRunStateResolver.resolvedRunState(
                apiRunState: "running",
                recentRunId: nil,
                committedState: GaryxTranscriptRunState()
            ),
            "idle"
        )
    }

    func testRunStateResolverDoesNotInventRunningFromNonRunningAPIState() {
        XCTAssertEqual(
            GaryxThreadSummaryRunStateResolver.resolvedRunState(
                apiRunState: "completed",
                recentRunId: "run-1",
                committedState: nil
            ),
            "completed"
        )
        XCTAssertEqual(
            GaryxThreadSummaryRunStateResolver.resolvedRunState(
                apiRunState: "idle",
                recentRunId: nil,
                committedState: nil
            ),
            "idle"
        )
        XCTAssertNil(
            GaryxThreadSummaryRunStateResolver.resolvedRunState(
                apiRunState: nil,
                recentRunId: nil,
                committedState: nil
            )
        )
    }

    func testSidebarThreadPresentationUsesWorkspaceSubtitleAndRunningState() {
        let thread = makeThread(
            title: "",
            workspacePath: "/workspace/project-alpha",
            runState: "running"
        )

        let presentation = GaryxSidebarThreadRowPresentation(
            thread: thread,
            isSelected: true,
            isPinned: true,
            trailingTimestamp: "now"
        )

        XCTAssertEqual(presentation.title, "Untitled")
        XCTAssertEqual(presentation.subtitle, "project-alpha")
        XCTAssertEqual(presentation.trailingTimestamp, "now")
        XCTAssertTrue(presentation.isSelected)
        XCTAssertTrue(presentation.isPinned)
        XCTAssertTrue(presentation.isRunning)
    }

    func testSidebarThreadPresentationJoinsWorkspaceAndCompactedPreview() {
        let thread = makeThread(
            workspacePath: "/workspace/project-alpha",
            lastMessagePreview: "line one\n  line two"
        )

        let presentation = GaryxSidebarThreadRowPresentation(
            thread: thread,
            isSelected: false,
            isPinned: false,
            trailingTimestamp: nil
        )

        XCTAssertEqual(presentation.subtitle, "project-alpha \u{00B7} line one line two")
    }

    func testSidebarThreadPresentationUsesPreviewWithoutWorkspace() {
        let thread = makeThread(lastMessagePreview: "hello there")

        let presentation = GaryxSidebarThreadRowPresentation(
            thread: thread,
            isSelected: false,
            isPinned: false,
            trailingTimestamp: nil
        )

        XCTAssertEqual(presentation.subtitle, "hello there")
    }

    func testAutomationDraftRequiresEitherAgentWorkspaceOrExistingThread() {
        var draft = GaryxAutomationDraft()
        draft.label = "Daily summary"
        draft.prompt = "Summarize updates"

        XCTAssertFalse(draft.canSubmit(workspacePaths: ["/workspace"], threadOptions: []))

        draft.agentTargetId = "agent-1"
        XCTAssertTrue(draft.canSubmit(workspacePaths: ["/workspace"], threadOptions: []))

        draft.targetsExistingThread = true
        XCTAssertFalse(draft.canSubmit(workspacePaths: ["/workspace"], threadOptions: []))

        draft.targetThreadId = "thread-1"
        XCTAssertTrue(draft.canSubmit(workspacePaths: [], threadOptions: []))
    }

    func testAutomationDraftEnsuresThreadWorkspaceSelection() {
        var draft = GaryxAutomationDraft()
        draft.targetsExistingThread = true

        draft.ensureTargetSelection(
            workspacePaths: [],
            threadOptions: [makeThread(id: "thread-1", workspacePath: "/workspace/project-alpha")]
        )

        XCTAssertEqual(draft.targetThreadId, "thread-1")
        XCTAssertEqual(draft.workspacePath, "/workspace/project-alpha")
    }

    func testAutomationDraftPreservesMissingWorkspacePath() {
        var draft = GaryxAutomationDraft()
        draft.label = "Daily summary"
        draft.prompt = "Summarize updates"
        draft.agentTargetId = "agent-1"
        draft.workspacePath = "/workspace/missing-current"

        XCTAssertEqual(
            draft.effectiveWorkspacePath(workspacePaths: ["/workspace/known"]),
            "/workspace/missing-current"
        )
        XCTAssertTrue(draft.canSubmit(workspacePaths: ["/workspace/known"], threadOptions: []))

        draft.ensureTargetSelection(workspacePaths: ["/workspace/known"], threadOptions: [])

        XCTAssertEqual(draft.workspacePath, "/workspace/missing-current")
    }

    func testAutomationScheduleDraftRoundTripsWeekdaysAndMonthlyClamp() {
        let weekdays = GaryxAutomationScheduleDraft(
            schedule: .daily(time: "09:30", weekdays: ["mo", "tu", "we", "th", "fr"], timezone: "UTC")
        )

        XCTAssertEqual(weekdays.repeatOption, .weekdays)
        XCTAssertEqual(weekdays.timeString, "09:30")
        XCTAssertEqual(weekdays.schedule, .daily(time: "09:30", weekdays: ["mo", "tu", "we", "th", "fr"], timezone: "UTC"))

        let monthly = GaryxAutomationScheduleDraft(
            schedule: GaryxAutomationSchedule(kind: .monthly, time: "08:00", timezone: "UTC", day: 99)
        )

        XCTAssertEqual(monthly.repeatOption, .monthly)
        XCTAssertEqual(monthly.schedule, .monthly(day: 31, time: "08:00", timezone: "UTC"))
    }

    private func makeThread(
        id: String = "thread-1",
        title: String = "Thread",
        workspacePath: String? = nil,
        activeRunId: String? = nil,
        runState: String? = nil,
        lastMessagePreview: String = ""
    ) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: id,
            title: title,
            createdAt: nil,
            updatedAt: nil,
            lastMessagePreview: lastMessagePreview,
            workspacePath: workspacePath,
            messageCount: nil,
            agentId: nil,
            teamId: nil,
            teamName: nil,
            providerType: nil,
            recentRunId: nil,
            activeRunId: activeRunId,
            runState: runState,
            worktreePath: nil
        )
    }
}
