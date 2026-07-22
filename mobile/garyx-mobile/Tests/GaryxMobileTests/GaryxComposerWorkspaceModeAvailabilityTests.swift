import XCTest
@testable import GaryxMobile

/// Workspace-mode availability is draft-scoped. These tests reproduce the
/// regression where a locally dispatched run in another thread hid the
/// Local/Worktree strip and silently refused mode changes for a fresh
/// new-thread draft (the strip's availability was gated on the global
/// `hasLocalActiveRun` instead of the draft's own state).
@MainActor
final class GaryxComposerWorkspaceModeAvailabilityTests: XCTestCase {
    private let gitWorkspacePath = "/Users/test/repos/sample"

    private func makeModel() throws -> (GaryxMobileModel, UserDefaults) {
        let suiteName = "workspace-mode-availability-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suiteName))
        addTeardownBlock {
            defaults.removePersistentDomain(forName: suiteName)
        }
        return (GaryxMobileModel(defaults: defaults), defaults)
    }

    /// Seeds a git workspace selection whose git status is already cached, so
    /// selection does not schedule a network probe.
    private func seedGitWorkspaceDraft(_ model: GaryxMobileModel) {
        model.workspaceGitStatuses[gitWorkspacePath] = GaryxWorkspaceGitStatus(
            workspaceDir: gitWorkspacePath,
            isGitRepo: true,
            repoRoot: gitWorkspacePath,
            currentBranch: "main",
            isDirty: false
        )
        model.selectDraftWorkspace(gitWorkspacePath)
    }

    /// Marks a locally dispatched run as active in an unrelated thread.
    private func startLocalRunElsewhere(_ model: GaryxMobileModel) {
        XCTAssertTrue(
            model.runTracker.beginLocalDispatch(
                threadId: "thread-elsewhere",
                intentId: "intent-elsewhere",
                text: "long agent run"
            )
        )
        model.runTracker.confirmChatStartAccepted(
            requestedThreadId: "thread-elsewhere",
            acceptedThreadId: "thread-elsewhere",
            intentId: "intent-elsewhere",
            runId: "run-elsewhere"
        )
    }

    // MARK: Regression repro

    func testActiveRunInAnotherThreadDoesNotLockDraftWorkspaceMode() throws {
        let (model, _) = try makeModel()
        seedGitWorkspaceDraft(model)
        startLocalRunElsewhere(model)

        // Repro precondition: the app is tracking a locally started run.
        XCTAssertNotNil(model.activeRunThreadId)
        XCTAssertTrue(model.isSending)

        // The strip must stay available for the independent draft…
        XCTAssertTrue(
            model.canChangeNewThreadWorkspaceMode,
            "a run in another thread must not hide the workspace-mode strip"
        )

        // …and worktree mode must be selectable.
        model.setNewThreadWorkspaceMode("worktree")
        XCTAssertTrue(
            model.newThreadUsesWorktree,
            "a run in another thread must not refuse worktree mode"
        )
    }

    // MARK: Draft-scoped guards stay intact

    func testModeIsUnavailableWithoutAWorkspaceSelection() throws {
        let (model, _) = try makeModel()
        model.selectDraftNoWorkspace()

        XCTAssertFalse(model.canChangeNewThreadWorkspaceMode)
        model.setNewThreadWorkspaceMode("worktree")
        XCTAssertFalse(model.newThreadUsesWorktree)
    }

    func testNonGitWorkspaceRefusesWorktreeMode() throws {
        let (model, _) = try makeModel()
        let plainPath = "/Users/test/documents/notes"
        model.workspaceGitStatuses[plainPath] = GaryxWorkspaceGitStatus(
            workspaceDir: plainPath,
            isGitRepo: false
        )
        model.selectDraftWorkspace(plainPath)

        XCTAssertTrue(model.canChangeNewThreadWorkspaceMode)
        model.setNewThreadWorkspaceMode("worktree")
        XCTAssertFalse(
            model.newThreadUsesWorktree,
            "worktree mode stays gated on the workspace being a git repo"
        )
    }

    func testSelectedThreadContextRefusesDraftModeChanges() throws {
        let (model, _) = try makeModel()
        seedGitWorkspaceDraft(model)
        model.selectedThread = GaryxThreadSummary(
            id: "thread-open",
            title: "Open Thread",
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
            worktreePath: nil
        )

        XCTAssertFalse(model.canChangeNewThreadWorkspaceMode)
        model.setNewThreadWorkspaceMode("worktree")
        XCTAssertFalse(model.newThreadUsesWorktree)
    }
}
